//! Cancel after browser-applied native input, then reuse the same tool/session.
use async_trait::async_trait;
use roder_api::{
    policy_mode::PolicyMode,
    tools::{ToolCall, ToolExecutionContext, ToolExecutor},
};
use roder_ext_chrome::{
    ComputerTool,
    direct::{
        DirectBinding, DirectGuard, DirectLease, DirectSession, DirectStep, DirectTab, OpenGuard,
    },
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
#[path = "../examples/native_computer/support.rs"]
#[allow(dead_code)]
mod support;
include!("support/cdp_relay.rs");
struct FixedBinding(DirectTab);
#[async_trait]
impl DirectBinding for FixedBinding {
    async fn lease(
        &self,
        _: &ToolExecutionContext,
        _: &ToolCall,
    ) -> Result<Box<dyn DirectLease>, String> {
        Ok(Box::new(FixedBinding(self.0.clone())))
    }
}
#[async_trait]
impl DirectLease for FixedBinding {
    fn tab(&self) -> DirectTab {
        self.0.clone()
    }
    fn guard(&self) -> Arc<dyn DirectGuard> {
        Arc::new(OpenGuard)
    }
    fn may_authorize(&self) -> bool {
        false
    }
    async fn finish(self: Box<Self>, _: &DirectStep, _: &str) {}
}
fn call(id: &str, actions: Value) -> ToolCall {
    let args = json!({"actions":actions});
    ToolCall {
        id: id.into(),
        name: "computer".into(),
        arguments: args.clone(),
        raw_arguments: args.to_string(),
        thread_id: "native-cancel".into(),
        turn_id: "turn".into(),
    }
}
async fn evaluate(tab: &DirectTab, expression: &str) -> Value {
    let DirectTab::Page { websocket, .. } = tab else {
        panic!("page expected")
    };
    tokio::time::timeout(Duration::from_secs(5),async {
        let(mut socket,_)=tokio_tungstenite::connect_async(websocket).await.unwrap();
        socket.send(Message::Text(json!({"id":1,"method":"Runtime.evaluate","params":{"expression":expression,"returnByValue":true}}).to_string().into())).await.unwrap();
        while let Some(message)=socket.next().await {
            if let Message::Text(text)=message.unwrap(){let data:Value=serde_json::from_str(&text).unwrap();if data["id"]==1{return data["result"]["result"]["value"].clone();}}
        }
        panic!("eval disconnected")
    }).await.unwrap()
}
#[tokio::test]
async fn cancelled_native_calls_release_input_and_masks_before_next_call() {
    let Some(browser) = support::Browser::start().await.unwrap() else {
        return;
    };
    let fixture = support::Fixture::start().await.unwrap();
    let pages: Value = reqwest::get(format!("{}/json", browser.endpoint))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let page = pages
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["type"] == "page")
        .unwrap();
    let tab = DirectTab::Page {
        websocket: page["webSocketDebuggerUrl"].as_str().unwrap().into(),
        target_id: page["id"].as_str().unwrap().into(),
    };
    let mut session = DirectSession::attach(&tab, Arc::new(OpenGuard), false)
        .await
        .unwrap();
    assert!(
        !session
            .run("navigate", &json!({"url":fixture.url}))
            .await
            .is_error
    );
    evaluate(&tab,"window.heldKeys=[];window.dragHeld=false;document.addEventListener('keydown',e=>heldKeys.push(e.key));document.addEventListener('keyup',e=>heldKeys=heldKeys.filter(k=>k!==e.key));document.addEventListener('mousedown',()=>dragHeld=true);document.addEventListener('mouseup',()=>dragHeld=false);true").await;
    for (fault, actions, check) in [
        (
            Fault::Key,
            json!([{"type":"keypress","keys":["CTRL","ARROWDOWN"]}]),
            "heldKeys.length===0",
        ),
        (
            Fault::Screenshot,
            json!([{"type":"screenshot"}]),
            "![...document.documentElement.children].some(e=>e.style.zIndex==='2147483647')",
        ),
        (
            Fault::Drag,
            json!([{"type":"drag","path":[{"x":40,"y":300},{"x":80,"y":320},{"x":180,"y":300}]}]),
            "dragHeld===false",
        ),
    ] {
        let (routed, applied, relay) = relay(&tab, fault).await;
        let tool = Arc::new(ComputerTool::new(Arc::new(FixedBinding(routed))));
        let ctx = ToolExecutionContext::new("native-cancel", "turn", PolicyMode::Bypass);
        let running = {
            let tool = tool.clone();
            let ctx = ctx.clone();
            tokio::spawn(
                async move { tool.execute(ctx, call("cancelled", actions)).await.unwrap() },
            )
        };
        tokio::time::timeout(Duration::from_secs(8), applied)
            .await
            .unwrap()
            .unwrap();
        if matches!(fault, Fault::Drag) {
            let result = running.await.unwrap();
            assert!(result.is_error && result.text.contains("fixture drag failure"));
        } else {
            running.abort();
            assert!(running.await.unwrap_err().is_cancelled());
        }
        // The cached session waits for cleanup. No separate grace period.
        let resumed = tool
            .execute(ctx, call("resumed", json!([{"type":"screenshot"}])))
            .await
            .unwrap();
        assert!(!resumed.is_error, "{}", resumed.text);
        assert_eq!(
            evaluate(&tab, check).await,
            true,
            "native cancellation left input or masks behind"
        );
        relay.abort();
    }
}
