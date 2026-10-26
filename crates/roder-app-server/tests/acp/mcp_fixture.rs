use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub struct Server {
    url: String,
    pub calls: Arc<Mutex<Vec<Value>>>,
    pub closes: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub fn config(&self) -> Value {
        json!([{"type":"http","name":"demo","url":self.url,"headers":[{"name":"Authorization","value":"Bearer fixture-secret"}]}])
    }

    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(vec![]));
        let closes = Arc::new(AtomicUsize::new(0));
        let recorded = calls.clone();
        let deleted = closes.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let calls = recorded.clone();
                let closes = deleted.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let header_end = loop {
                        let mut chunk = [0; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert_ne!(count, 0);
                        bytes.extend_from_slice(&chunk[..count]);
                        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                            break index + 4;
                        }
                    };
                    let headers = String::from_utf8(bytes[..header_end].to_vec())
                        .unwrap()
                        .to_ascii_lowercase();
                    assert!(headers.contains("authorization: bearer fixture-secret"));
                    if headers.starts_with("delete ") {
                        closes.fetch_add(1, Ordering::SeqCst);
                        socket
                            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                            .await
                            .unwrap();
                        return;
                    }
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    while bytes.len() < header_end + length {
                        let mut chunk = [0; 4096];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert_ne!(count, 0);
                        bytes.extend_from_slice(&chunk[..count]);
                    }
                    let request: Value =
                        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                    if request["method"] == "notifications/initialized" {
                        socket.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                        return;
                    }
                    let result = match request["method"].as_str().unwrap() {
                        "initialize" => {
                            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                        }
                        "tools/list" => {
                            json!({"tools":[{"name":"echo","description":"echo","inputSchema":{"type":"object","properties":{"value":{"type":"string"}}}}]})
                        }
                        "tools/call" => {
                            calls
                                .lock()
                                .await
                                .push(request["params"]["arguments"].clone());
                            json!({"content":[{"type":"text","text":"echo:hello"}],"isError":false})
                        }
                        other => panic!("unexpected MCP request {other}"),
                    };
                    let response =
                        json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
                    let wire = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: fixture-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    );
                    socket.write_all(wire.as_bytes()).await.unwrap();
                });
            }
        });
        Self {
            url,
            calls,
            closes,
            task,
        }
    }
}

pub const STDIO_SERVER: &str = r#"
import sys,json,os
for line in sys.stdin:
    request=json.loads(line)
    if 'id' not in request: continue
    method=request['method']
    if method=='initialize': result={'protocolVersion':'2025-06-18','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
    elif method=='tools/list': result={'tools':[{'name':'echo','description':'echo','inputSchema':{'type':'object','properties':{'value':{'type':'string'}}}}]}
    elif method=='tools/call': result={'content':[{'type':'text','text':os.getcwd()+' '+os.environ['ACP_SENTINEL']}],'isError':False}
    else: raise ValueError(method)
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
"#;
