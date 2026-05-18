use std::sync::Arc;

use roder_api::extension::ExtensionRegistry;
use roder_api::interactive::{
    HandlerOutcome, InteractiveEvent, InteractiveRegion, InteractiveRegionHandler, RegionId,
};

#[derive(Clone, Default)]
pub struct RegionHandlerDispatcher {
    handlers: Vec<Arc<dyn InteractiveRegionHandler>>,
}

impl RegionHandlerDispatcher {
    pub fn new(handlers: Vec<Arc<dyn InteractiveRegionHandler>>) -> Self {
        Self { handlers }
    }

    pub fn from_registry(registry: &ExtensionRegistry) -> Self {
        Self::new(registry.interactive_region_handlers.clone())
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    pub async fn dispatch(
        &self,
        event: InteractiveEvent,
        region: &InteractiveRegion,
    ) -> anyhow::Result<HandlerOutcome> {
        let kind_name = region.kind.kind_name();
        for handler in &self.handlers {
            if !handler.kinds().contains(&kind_name) {
                continue;
            }
            match handler.handle(event.clone(), region).await? {
                HandlerOutcome::Passthrough => {}
                outcome => return Ok(outcome),
            }
        }
        Ok(HandlerOutcome::Passthrough)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedInteractiveEvent {
    pub event: InteractiveEvent,
    pub region: Option<RegionId>,
    pub outcome: HandlerOutcome,
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::interactive::{
        HoverCursor, InteractiveModifiers, InteractiveMouseButton, RegionKind, RegionRect,
    };

    use super::*;

    struct FakeHandler {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl InteractiveRegionHandler for FakeHandler {
        fn id(&self) -> String {
            "fake-url-handler".to_string()
        }

        fn kinds(&self) -> &[&'static str] {
            &["Url"]
        }

        async fn handle(
            &self,
            _event: InteractiveEvent,
            _region: &InteractiveRegion,
        ) -> anyhow::Result<HandlerOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HandlerOutcome::Consumed)
        }
    }

    #[tokio::test]
    async fn dispatcher_routes_matching_region_kind_to_extension_handler() {
        let calls = Arc::new(AtomicUsize::new(0));
        let dispatcher = RegionHandlerDispatcher::new(vec![Arc::new(FakeHandler {
            calls: Arc::clone(&calls),
        })]);
        let region = InteractiveRegion {
            id: "url-1".to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
            },
            z: 0,
            kind: RegionKind::Url("https://example.com".to_string()),
            hover_cursor: HoverCursor::Pointer,
            keyboard_binding: None,
        };

        let outcome = dispatcher
            .dispatch(
                InteractiveEvent::Click {
                    region: "url-1".to_string(),
                    modifiers: InteractiveModifiers::default(),
                    button: InteractiveMouseButton::Left,
                },
                &region,
            )
            .await
            .unwrap();

        assert_eq!(outcome, HandlerOutcome::Consumed);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatcher_loads_extension_handlers_from_registry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = ExtensionRegistryBuilder::new();
        builder.interactive_region_handler(Arc::new(FakeHandler {
            calls: Arc::clone(&calls),
        }));
        let registry = builder.build().unwrap();
        let dispatcher = RegionHandlerDispatcher::from_registry(&registry);
        let region = InteractiveRegion {
            id: "url-1".to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
            },
            z: 0,
            kind: RegionKind::Url("https://example.com".to_string()),
            hover_cursor: HoverCursor::Pointer,
            keyboard_binding: None,
        };

        let outcome = dispatcher
            .dispatch(
                InteractiveEvent::Click {
                    region: "url-1".to_string(),
                    modifiers: InteractiveModifiers::default(),
                    button: InteractiveMouseButton::Left,
                },
                &region,
            )
            .await
            .unwrap();

        assert_eq!(outcome, HandlerOutcome::Consumed);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
