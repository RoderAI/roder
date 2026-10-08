#!/usr/bin/env python3
"""Native GTK input fixture; oracle records only application-observed events."""
import json
from pathlib import Path
import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk, Gdk, GLib

ORACLE = Path('/tmp/roder-cua-input-oracle.json')
state = {'text':'', 'buttons':[], 'double_clicks':0, 'right_clicks':0,
         'drags':0, 'scrolls':0, 'motions':0, 'chords':0, 'dialog_open':False,
         'dialog_text':'', 'focus':False, 'focus_seen':False, 'held_button':False, 'geometry':{}, 'targets':{}}
window = Gtk.Window(title='Roder Cua Input Fixture')
window.set_default_size(560, 430)
window.connect('destroy', Gtk.main_quit)
box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
box.set_border_width(12)
window.add(box)
entry = Gtk.Entry()
entry.get_accessible().set_name('Unicode input')
box.pack_start(entry, False, False, 0)
pointer = Gtk.DrawingArea()
pointer.set_size_request(500, 150)
pointer.get_accessible().set_name('Pointer target')
pointer.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK |
                   Gdk.EventMask.POINTER_MOTION_MASK | Gdk.EventMask.SCROLL_MASK)
box.pack_start(pointer, False, False, 0)
scroller = Gtk.ScrolledWindow()
scroller.set_size_request(500, 110)
text = Gtk.TextView()
text.set_editable(False)
text.get_buffer().set_text('\n'.join('Scroll line %d' % i for i in range(150)))
text.get_accessible().set_name('Scroll target')
scroller.add(text)
box.pack_start(scroller, True, True, 0)
button = Gtk.Button(label='Open dialog')
box.pack_start(button, False, False, 0)
pressed = None


def save():
    if window.get_realized():
        state['focus_seen'] = state['focus_seen'] or window.is_active()
        position, size = window.get_position(), window.get_size()
        state['geometry'] = {'x':position.root_x, 'y':position.root_y,
                             'width':size.width, 'height':size.height}
        for name, widget in [('pointer',pointer), ('scroll',scroller), ('entry',entry), ('dialog_button',button)]:
            xy = widget.translate_coordinates(window,0,0)
            allocation = widget.get_allocation()
            if xy:
                state['targets'][name] = {'x':xy[0], 'y':xy[1], 'width':allocation.width, 'height':allocation.height}
    tmp = ORACLE.with_suffix('.tmp')
    tmp.write_text(json.dumps(state))
    tmp.replace(ORACLE)
    return True


def press(widget, event):
    global pressed
    state['buttons'].append(int(event.button))
    if event.button==3: state['right_clicks'] += 1
    if event.type==Gdk.EventType._2BUTTON_PRESS: state['double_clicks'] += 1
    pressed = (event.x,event.y)
    state['held_button'] = True
    save()
    return True


def release(widget, event):
    global pressed
    if pressed and abs(event.x-pressed[0])+abs(event.y-pressed[1])>60:
        state['drags'] += 1
    pressed = None
    state['held_button'] = False
    save()
    return True


def motion(widget, event):
    state['motions'] += 1
    save()
    return False


def scroll(widget, event):
    state['scrolls'] += 1
    save()
    return False


def key(widget, event):
    if event.state & Gdk.ModifierType.CONTROL_MASK and event.keyval==Gdk.KEY_k:
        state['chords'] += 1
        save()
        return True
    return False


def open_dialog(widget):
    state['dialog_open'] = True
    dialog = Gtk.Dialog(title='Roder Cua Dialog', transient_for=window, modal=False)
    dialog.set_default_size(360,120)
    dialog_entry = Gtk.Entry()
    dialog_entry.get_accessible().set_name('Dialog input')
    dialog.get_content_area().pack_start(dialog_entry,False,False,8)
    def changed(widget):
        state['dialog_text'] = widget.get_text()
        save()
    def destroyed(widget):
        state['dialog_open'] = False
        save()
    dialog_entry.connect('changed', changed)
    dialog.connect('destroy',destroyed)
    dialog.show_all()
    save()


def changed(widget):
    state['text'] = widget.get_text()
    save()


def focused(widget, event):
    state['focus'] = bool(event.in_)
    save()
    return False

entry.connect('changed',changed)
window.connect('key-press-event',key)
window.connect('focus-in-event',focused)
window.connect('focus-out-event',focused)
pointer.connect('button-press-event',press)
pointer.connect('button-release-event',release)
pointer.connect('motion-notify-event',motion)
pointer.connect('scroll-event',scroll)
text.connect('scroll-event',scroll)
button.connect('clicked',open_dialog)
window.show_all()
GLib.timeout_add(200,save)
Gtk.main()
