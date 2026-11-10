use serde_json::{Value, json};
use std::ffi::OsString;
use std::net::{Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

const BROWSER_ENV: &str = "PHONEPAD_CHROMIUM";
const DEFAULT_BROWSER: &str = "chromium";
const PORT_FILE: &str = "DevToolsActivePort";
const START_TIMEOUT: Duration = Duration::from_secs(15);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);
const CLOSE_PATIENCE: Duration = Duration::from_secs(5);
const CONNECTED: &str = "\"status\":\"connected\"";
const LOGIN_HEADER: &str = "Tailscale-User-Login";
pub const PHONE: Phone = Phone {
    width: 412,
    height: 915,
    scale: 2.625,
};
const SOFTWARE_DECODE: &str = "(() => { const configure = VideoDecoder.prototype.configure; VideoDecoder.prototype.configure = function (config) { return configure.call(this, { ...config, hardwareAcceleration: \"prefer-software\" }); }; })();";
const VIEW_PROBE: &str = "JSON.stringify({ status: document.getElementById(\"status\").textContent, pad: [pad.getBoundingClientRect().width, pad.getBoundingClientRect().height], zoom })";

pub struct Phone {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

pub struct Browser {
    child: Child,
    socket: WebSocket<TcpStream>,
    session: String,
    next_id: u64,
}

pub fn endpoint(port_file: &str) -> Option<(u16, String)> {
    match port_file.lines().collect::<Vec<&str>>().as_slice() {
        [port, path] => Some((port.parse().ok()?, (*path).to_owned())),
        _ => None,
    }
}

fn devtools(profile: &Path) -> Result<(u16, String), String> {
    let file: PathBuf = profile.join(PORT_FILE);
    let start: Instant = Instant::now();
    while start.elapsed() < START_TIMEOUT {
        if let Some(found) = std::fs::read_to_string(&file)
            .ok()
            .as_deref()
            .and_then(endpoint)
        {
            return Ok(found);
        }
        thread::sleep(POLL);
    }
    Err(format!(
        "{} did not appear within {START_TIMEOUT:?}",
        file.display()
    ))
}

pub fn reply_for(message: &Value, id: u64) -> Option<Result<Value, String>> {
    (message["id"].as_u64() == Some(id)).then(|| match message.get("error") {
        Some(error) => Err(error.to_string()),
        None => Ok(message["result"].clone()),
    })
}

impl Browser {
    pub fn launch(profile: &Path) -> Result<Browser, String> {
        let program: OsString =
            std::env::var_os(BROWSER_ENV).unwrap_or_else(|| OsString::from(DEFAULT_BROWSER));
        let child: Child = Command::new(&program)
            .arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--mute-audio",
            ])
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", program.to_string_lossy()))?;
        let mut browser: Browser = Browser {
            child,
            socket: Browser::connect(profile)?,
            session: String::new(),
            next_id: 0,
        };
        let target: Value = browser.call("Target.createTarget", json!({ "url": "about:blank" }))?;
        let attached: Value = browser.call(
            "Target.attachToTarget",
            json!({ "targetId": target["targetId"], "flatten": true }),
        )?;
        browser.session = attached["sessionId"]
            .as_str()
            .ok_or("attachToTarget named no session")?
            .to_owned();
        Ok(browser)
    }

    fn connect(profile: &Path) -> Result<WebSocket<TcpStream>, String> {
        let (port, path): (u16, String) = devtools(profile)?;
        let url: String = format!("ws://127.0.0.1:{port}{path}");
        let stream: TcpStream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .map_err(|e| format!("devtools {url}: {e}"))?;
        stream
            .set_read_timeout(Some(REPLY_TIMEOUT))
            .map_err(|e| e.to_string())?;
        let (socket, _): (WebSocket<TcpStream>, _) = tungstenite::client(url.as_str(), stream)
            .map_err(|e| format!("devtools {url}: {e}"))?;
        Ok(socket)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id: u64 = self.next_id;
        let mut request: Value = json!({ "id": id, "method": method, "params": params });
        if !self.session.is_empty() {
            request["sessionId"] = Value::String(self.session.clone());
        }
        self.socket
            .send(Message::text(request.to_string()))
            .map_err(|e| format!("{method}: {e}"))?;
        loop {
            let message: Message = self.socket.read().map_err(|e| format!("{method}: {e}"))?;
            let parsed: Option<Value> = match &message {
                Message::Text(text) => serde_json::from_str(text.as_str()).ok(),
                Message::Binary(_)
                | Message::Ping(_)
                | Message::Pong(_)
                | Message::Close(_)
                | Message::Frame(_) => None,
            };
            if let Some(reply) = parsed.and_then(|value: Value| reply_for(&value, id)) {
                return reply.map_err(|error: String| format!("{method}: {error}"));
            }
        }
    }

    pub fn open(&mut self, url: &str, login: &str) -> Result<(), String> {
        self.call("Network.enable", json!({}))?;
        self.call(
            "Network.setExtraHTTPHeaders",
            json!({ "headers": { LOGIN_HEADER: login } }),
        )?;
        self.call(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": PHONE.width, "height": PHONE.height, "deviceScaleFactor": PHONE.scale, "mobile": true }),
        )?;
        self.call("Page.enable", json!({}))?;
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": SOFTWARE_DECODE }),
        )?;
        self.call("Page.navigate", json!({ "url": url }))?;
        let start: Instant = Instant::now();
        while !self.view()?.contains(CONNECTED) {
            if start.elapsed() > START_TIMEOUT {
                return Err(format!(
                    "the page did not connect within {START_TIMEOUT:?}: {}",
                    self.view()?
                ));
            }
            thread::sleep(POLL);
        }
        self.call("Network.disable", json!({}))?;
        Ok(())
    }

    pub fn view(&mut self) -> Result<String, String> {
        let evaluated: Value = self.call(
            "Runtime.evaluate",
            json!({ "expression": VIEW_PROBE, "returnByValue": true }),
        )?;
        Ok(evaluated["result"]["value"]
            .as_str()
            .map_or_else(|| evaluated.to_string(), str::to_owned))
    }

    pub fn preload(&mut self, source: &str) -> Result<(), String> {
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": source }),
        )
        .map(drop)
    }

    pub fn evaluate(&mut self, expression: &str) -> Result<String, String> {
        let evaluated: Value = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )?;
        evaluated["result"]["value"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{expression} gave no string: {evaluated}"))
    }

    pub fn touch(&mut self, kind: &str, finger: Option<(f64, f64)>) -> Result<(), String> {
        let points: Vec<Value> = finger
            .into_iter()
            .map(|(x, y): (f64, f64)| json!({ "x": x, "y": y, "id": 0 }))
            .collect();
        self.call(
            "Input.dispatchTouchEvent",
            json!({ "type": kind, "touchPoints": points }),
        )
        .map(drop)
    }

    pub fn close(mut self) {
        self.session.clear();
        if let Err(error) = self.call("Browser.close", json!({})) {
            eprintln!("Browser.close: {error}");
        }
        let start: Instant = Instant::now();
        while start.elapsed() < CLOSE_PATIENCE && matches!(self.child.try_wait(), Ok(None)) {
            thread::sleep(POLL);
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            if let Err(error) = self.child.kill() {
                eprintln!("kill chromium {}: {error}", self.child.id());
            }
            if let Err(error) = self.child.wait() {
                eprintln!("wait chromium {}: {error}", self.child.id());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{endpoint, reply_for};
    use serde_json::json;

    #[test]
    fn the_port_file_names_the_port_and_the_browser_path() {
        assert_eq!(
            endpoint("35625\n/devtools/browser/2fcf\n"),
            Some((35625, String::from("/devtools/browser/2fcf")))
        );
    }

    #[test]
    fn a_half_written_port_file_names_nothing() {
        assert_eq!(endpoint("35625\n"), None);
    }

    #[test]
    fn an_empty_port_file_names_nothing() {
        assert_eq!(endpoint(""), None);
    }

    #[test]
    fn a_non_numeric_port_names_nothing() {
        assert_eq!(endpoint("port\n/devtools/browser/x"), None);
    }

    #[test]
    fn another_id_is_not_the_reply() {
        assert_eq!(reply_for(&json!({ "id": 2, "result": {} }), 1), None);
    }

    #[test]
    fn an_event_is_not_the_reply() {
        assert_eq!(
            reply_for(&json!({ "method": "Page.loadEventFired" }), 1),
            None
        );
    }

    #[test]
    fn the_reply_carries_its_result() {
        assert_eq!(
            reply_for(&json!({ "id": 1, "result": { "sessionId": "s" } }), 1),
            Some(Ok(json!({ "sessionId": "s" })))
        );
    }

    #[test]
    fn an_error_reply_is_an_error() {
        assert_eq!(
            reply_for(&json!({ "id": 1, "error": { "code": -32000 } }), 1),
            Some(Err(String::from(r#"{"code":-32000}"#)))
        );
    }
}
