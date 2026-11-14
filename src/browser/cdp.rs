use anyhow::{anyhow, bail, Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

pub struct Cdp {
    tx: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next_id: AtomicU64,
    child: Mutex<Option<tokio::process::Child>>,
}

pub struct Browser {
    pub cdp: Arc<Cdp>,
    pub user_data_dir: std::path::PathBuf,
}

impl Browser {
    pub async fn launch(
        executable: &Path,
        headless: bool,
        user_data_dir: &Path,
        proxy: Option<&str>,
    ) -> Result<Self> {
        std::fs::create_dir_all(user_data_dir)?;
        let mut cmd = tokio::process::Command::new(executable);
        cmd.arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-background-timer-throttling")
            .arg("--disk-cache-size=1048576")
            .arg("--media-cache-size=1048576")
            .arg("--disable-gpu-shader-disk-cache")
            .arg("--disable-infobars")
            .arg("--window-position=0,0")
            .arg("--ignore-certificate-errors")
            .arg("--ignore-certificate-errors-spki-list")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", user_data_dir.display()))
            .arg(
                "--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:126.0) Gecko/20100101 Firefox/126.0",
            );
        if headless {
            cmd.arg("--headless=new");
        }
        if let Some(proxy) = proxy {
            if !proxy.is_empty() {
                cmd.arg(format!("--proxy-server={proxy}"));
            }
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().context("launching the browser")?;

        let port_file = user_data_dir.join("DevToolsActivePort");
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut endpoint = None;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&port_file) {
                let mut lines = text.lines();
                if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                    endpoint = Some(format!("ws://127.0.0.1:{port}{path}"));
                    break;
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                bail!("browser exited early ({status})");
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let Some(endpoint) = endpoint else {
            let _ = child.kill().await;
            bail!("browser did not expose a DevTools endpoint");
        };

        let (ws, _) = tokio_tungstenite::connect_async(&endpoint)
            .await
            .with_context(|| format!("connecting to {endpoint}"))?;
        let (mut sink, mut stream) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
        });

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = Arc::clone(&pending);
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let Ok(message) = message else {
                    break;
                };
                if let Message::Text(text) = message {
                    let Ok(value) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
                        if let Some(sender) = pending_reader.lock().unwrap().remove(&id) {
                            if let Some(error) = value.get("error") {
                                let _ = sender.send(Err(anyhow!("CDP error: {error}")));
                            } else {
                                let _ = sender.send(Ok(value.get("result").cloned().unwrap_or(Value::Null)));
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            cdp: Arc::new(Cdp {
                tx,
                pending,
                next_id: AtomicU64::new(1),
                child: Mutex::new(Some(child)),
            }),
            user_data_dir: user_data_dir.to_path_buf(),
        })
    }

    pub async fn new_page(&self, url: &str) -> Result<Page> {
        let target = self
            .cdp
            .send(
                "Target.createTarget",
                json!({"url": "about:blank"}),
                None,
            )
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("no target id"))?
            .to_string();
        let attached = self
            .cdp
            .send(
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
                None,
            )
            .await?;
        let session_id = attached
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("no session id"))?
            .to_string();
        let page = Page {
            cdp: Arc::clone(&self.cdp),
            session_id,
            target_id,
        };
        page.send("Page.enable", json!({})).await?;
        page.send("Runtime.enable", json!({})).await?;
        page.navigate(url).await?;
        Ok(page)
    }

    pub async fn close(&self) {
        let _ = self.cdp.send("Browser.close", json!({}), None).await;
        let child = { self.cdp.child.lock().unwrap().take() };
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(child) = self.cdp.child.lock().unwrap().as_mut() {
            let _ = child.start_kill();
        }
    }
}

pub struct Page {
    cdp: Arc<Cdp>,
    session_id: String,
    target_id: String,
}

impl Cdp {
    pub async fn send(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = Value::from(session);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.tx
            .send(Message::Text(message.to_string().into()))
            .map_err(|_| anyhow!("browser connection closed"))?;
        match tokio::time::timeout(Duration::from_secs(40), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => bail!("browser command {method} was cancelled"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                bail!("browser command {method} timed out")
            }
        }
    }
}

impl Page {
    async fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.cdp.send(method, params, Some(&self.session_id)).await
    }

    pub async fn eval(&self, expression: &str) -> Result<Value> {
        let result = self
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                    "userGesture": true
                }),
            )
            .await?;
        if result.get("exceptionDetails").is_some() {
            let text = result
                .get("exceptionDetails")
                .and_then(|e| e.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("script error");
            bail!("page script failed: {text}");
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn navigate(&self, url: &str) -> Result<()> {
        self.send("Page.navigate", json!({"url": url})).await?;
        self.wait_ready().await
    }

    pub async fn url(&self) -> Result<String> {
        Ok(self.eval("location.href").await?.as_str().unwrap_or("").to_string())
    }

    pub async fn body_text(&self) -> Result<String> {
        Ok(self
            .eval("document.body ? document.body.innerText : ''")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string())
    }

    pub async fn wait_ready(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(value) = self.eval("document.readyState").await {
                if value.as_str() == Some("complete") {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Ok(())
    }

    pub async fn exists(&self, selector: &str) -> Result<bool> {
        let js = format!(
            "!!document.querySelector({})",
            serde_json::to_string(selector)?
        );
        Ok(self.eval(&js).await?.as_bool().unwrap_or(false))
    }

    pub async fn wait_selector(&self, selector: &str, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.exists(selector).await.unwrap_or(false) {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(false)
    }

    pub async fn wait_js(&self, expression: &str, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.eval(expression).await?.as_bool().unwrap_or(false) {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(false)
    }

    /// Set an input value and fire the events React listens for.
    pub async fn type_into(&self, selector: &str, text: &str) -> Result<()> {
        if !self.wait_selector(selector, Duration::from_secs(10)).await? {
            bail!("input {selector} did not appear");
        }
        let sel = serde_json::to_string(selector)?;
        let value = serde_json::to_string(text)?;
        let js = format!(
            r#"(() => {{
                const el = document.querySelector({sel});
                if (!el) return null;
                el.focus();
                const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
                const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
                setter.call(el, {value});
                el.dispatchEvent(new Event('input', {{bubbles: true}}));
                el.dispatchEvent(new Event('change', {{bubbles: true}}));
                el.dispatchEvent(new Event('blur', {{bubbles: true}}));
                return el.value;
            }})()"#
        );
        let result = self.eval(&js).await?;
        if result.as_str() != Some(text) {
            bail!("could not type into {selector}");
        }
        Ok(())
    }

    pub async fn click(&self, selector: &str) -> Result<bool> {
        let sel = serde_json::to_string(selector)?;
        let js = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return false; el.scrollIntoView({{block:'center'}}); el.click(); return true; }})()"
        );
        Ok(self.eval(&js).await?.as_bool().unwrap_or(false))
    }

    pub async fn click_text(&self, tag: &str, text: &str) -> Result<bool> {
        let tag = serde_json::to_string(tag)?;
        let text = serde_json::to_string(text)?;
        let js = format!(
            "(() => {{ const nodes = Array.from(document.querySelectorAll({tag})); const el = nodes.find(n => (n.innerText || '').trim() === {text}); if (!el) return false; el.scrollIntoView({{block:'center'}}); el.click(); return true; }})()"
        );
        Ok(self.eval(&js).await?.as_bool().unwrap_or(false))
    }

    pub async fn click_if_exists(&self, selector: &str) -> Result<bool> {
        if self.exists(selector).await? {
            return self.click(selector).await;
        }
        Ok(false)
    }

    pub async fn wait_url_not_contains(&self, needle: &str, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let url = self.url().await.unwrap_or_default();
            if !url.contains(needle) {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(false)
    }

    pub async fn close(&self) {
        let _ = self
            .cdp
            .send(
                "Target.closeTarget",
                json!({"targetId": self.target_id}),
                None,
            )
            .await;
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        let message = json!({
            "id": 0,
            "method": "Target.closeTarget",
            "params": {"targetId": self.target_id}
        });
        let _ = self
            .cdp
            .tx
            .send(Message::Text(message.to_string().into()));
    }
}
