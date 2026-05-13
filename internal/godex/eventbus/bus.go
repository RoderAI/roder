package eventbus

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"github.com/google/uuid"
)

const defaultSubscriberBuffer = 256

var ErrClosed = errors.New("event bus closed")

type Option func(*Bus)

func WithSubscriberBuffer(size int) Option {
	return func(b *Bus) {
		if size > 0 {
			b.subscriberBuffer = size
		}
	}
}

type subscription struct {
	id     string
	filter Filter
	ch     chan Event
}

type Bus struct {
	mu               sync.RWMutex
	subs             map[string]subscription
	closed           bool
	seq              atomic.Uint64
	subscriberBuffer int
}

func New(opts ...Option) *Bus {
	b := &Bus{
		subs:             make(map[string]subscription),
		subscriberBuffer: defaultSubscriberBuffer,
	}
	for _, opt := range opts {
		opt(b)
	}
	return b
}

func (b *Bus) Publish(ctx context.Context, event Event) Event {
	if event.ID == "" {
		event.ID = uuid.NewString()
	}
	if event.Time.IsZero() {
		event.Time = time.Now().UTC()
	}
	if event.Seq == 0 {
		event.Seq = b.seq.Add(1)
	}

	b.mu.RLock()
	if b.closed {
		b.mu.RUnlock()
		return event
	}
	subs := make([]subscription, 0, len(b.subs))
	for _, sub := range b.subs {
		if sub.filter.Match(event) {
			subs = append(subs, sub)
		}
	}
	b.mu.RUnlock()

	for _, sub := range subs {
		select {
		case <-ctx.Done():
			return event
		case sub.ch <- event:
		default:
			b.publishDrop(sub, event)
		}
	}
	return event
}

func (b *Bus) Subscribe(ctx context.Context, filter Filter) <-chan Event {
	ch := make(chan Event, b.subscriberBuffer)
	sub := subscription{id: uuid.NewString(), filter: filter, ch: ch}

	b.mu.Lock()
	if b.closed {
		close(ch)
		b.mu.Unlock()
		return ch
	}
	b.subs[sub.id] = sub
	b.mu.Unlock()

	go func() {
		<-ctx.Done()
		b.mu.Lock()
		if existing, ok := b.subs[sub.id]; ok {
			delete(b.subs, sub.id)
			close(existing.ch)
		}
		b.mu.Unlock()
	}()

	return ch
}

func (b *Bus) Await(ctx context.Context, filter Filter) (Event, error) {
	ch := b.Subscribe(ctx, filter)
	select {
	case ev, ok := <-ch:
		if !ok {
			return Event{}, ErrClosed
		}
		return ev, nil
	case <-ctx.Done():
		return Event{}, ctx.Err()
	}
}

func (b *Bus) Close() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return nil
	}
	b.closed = true
	for id, sub := range b.subs {
		delete(b.subs, id)
		close(sub.ch)
	}
	return nil
}

func (b *Bus) publishDrop(sub subscription, dropped Event) {
	drop := Event{
		ID:            uuid.NewString(),
		Seq:           b.seq.Add(1),
		Time:          time.Now().UTC(),
		SessionID:     dropped.SessionID,
		RunID:         dropped.RunID,
		Source:        SourceSystem,
		Kind:          KindSubscriberDrop,
		CorrelationID: dropped.CorrelationID,
	}
	drop.SetPayload(map[string]any{
		"subscriber_id": sub.id,
		"dropped_kind":  dropped.Kind,
		"dropped_seq":   dropped.Seq,
	})

	select {
	case sub.ch <- drop:
	default:
	}
}

func (b *Bus) String() string {
	b.mu.RLock()
	defer b.mu.RUnlock()
	return fmt.Sprintf("Bus{subscribers:%d}", len(b.subs))
}
