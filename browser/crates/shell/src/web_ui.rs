//! Trusted chrome dialogs and bounded desktop adapters. Never run shell source.
use crate::{
    omnibox::Omnibox,
    text::{Canvas, TextBuffer},
};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use web_api::{
    runtime::{Dialog, DialogKind},
    ApiError,
};
#[derive(Debug, Clone)]
pub enum Action {
    Permission,
    Clipboard(Option<String>),
}
pub enum Kind {
    Permission {
        request: u64,
        name: String,
        action: Action,
    },
    Dialog {
        kind: DialogKind,
        reply: Sender<Option<String>>,
    },
    Popup(url::Url),
    Notice,
    Settings,
}
pub struct WebDialog {
    pub origin: String,
    pub message: String,
    pub input: Omnibox,
    pub kind: Kind,
}
impl WebDialog {
    pub fn native(dialog: Dialog, reply: Sender<Option<String>>) -> Self {
        let mut input = Omnibox::new();
        input.set_text(&dialog.default.chars().take(4096).collect::<String>());
        Self {
            origin: dialog.origin,
            message: dialog.message.chars().take(4096).collect(),
            input,
            kind: Kind::Dialog {
                kind: dialog.kind,
                reply,
            },
        }
    }
    pub fn notice(message: impl Into<String>) -> Self {
        Self {
            origin: "Browser".into(),
            message: message.into(),
            input: Omnibox::new(),
            kind: Kind::Notice,
        }
    }
    pub fn rect(width: u32) -> (i32, i32, i32, i32) {
        (16, 64, (width as i32 - 32).max(40), 220)
    }
    pub fn draw(
        &self,
        fs: &mut cosmic_text::FontSystem,
        cache: &mut cosmic_text::SwashCache,
        canvas: &mut Canvas<'_>,
    ) {
        let (x, y, w, h) = Self::rect(canvas.w);
        canvas.fill_rect(x, y, w, h, 0xeef4ff);
        canvas.fill_rect(x, y, w, 3, 0x0b5dc2);
        let choices = match &self.kind {
            Kind::Permission { .. } => "[1] Once     [2] Always     [3] Never     [Esc] Deny",
            Kind::Dialog {
                kind: DialogKind::Alert,
                ..
            }
            | Kind::Notice => "[1 / Enter] OK",
            Kind::Dialog { .. } => "[1 / Enter] OK       [Esc] Cancel",
            Kind::Popup(_) => "[1 / Enter] Open new window       [Esc] Block",
            Kind::Settings => "[1] Revoke this origin     [2] Revoke all     [Esc] Close",
        };
        let input = if matches!(
            self.kind,
            Kind::Dialog {
                kind: DialogKind::Prompt,
                ..
            }
        ) {
            format!("\n> {}", self.input.text())
        } else {
            String::new()
        };
        let mut text = TextBuffer::with_metrics(fs, 16.0, cosmic_text::Wrap::Word);
        text.set_text(
            fs,
            &format!(
                "{}\n\n{}{}",
                self.origin,
                self.message.chars().take(400).collect::<String>(),
                input
            ),
        );
        text.set_size(fs, (w - 24).max(1) as f32, (h - 68) as f32);
        text.draw(fs, cache, canvas, x + 12, y + 12);
        canvas.fill_rect(x, y + h - 44, w, 44, 0xd6e5ff);
        let mut buttons = TextBuffer::with_metrics(fs, 15.0, cosmic_text::Wrap::Word);
        buttons.set_text(fs, choices);
        buttons.set_size(fs, (w - 24).max(1) as f32, 40.0);
        buttons.draw(fs, cache, canvas, x + 12, y + h - 38);
    }
}
/// Linux Wayland adapter; absence of the backend is an explicit failure.
pub fn clipboard(text: Option<String>) -> web_api::Result<String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(ApiError::new(
            "NotSupportedError",
            "Wayland clipboard unavailable",
        ));
    }
    let mut command = if text.is_some() {
        let mut c = Command::new("wl-copy");
        c.args(["--type", "text/plain;charset=utf-8"]);
        c
    } else {
        let mut c = Command::new("wl-paste");
        c.args(["--no-newline", "--type", "text/plain"]);
        c
    };
    let output = run(&mut command, text.as_deref().map(str::as_bytes))?;
    Ok(if text.is_some() {
        String::new()
    } else {
        String::from_utf8_lossy(&output).into_owned()
    })
}
pub fn notify(title: &str, body: &str) -> web_api::Result<()> {
    run(
        Command::new("notify-send").args(["--app-name=browser", "--", title, body]),
        None,
    )
    .map(|_| ())
}
pub fn spawn_notification(
    title: String,
    body: String,
    on_error: impl FnOnce(ApiError) + Send + 'static,
) -> web_api::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    reserve(&ACTIVE)?;
    let result = std::thread::Builder::new()
        .name("notification".into())
        .spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    ACTIVE.fetch_sub(1, Ordering::Relaxed);
                }
            }
            let _guard = Guard;
            if let Err(error) = notify(&title, &body) {
                on_error(error);
            }
        });
    if let Err(error) = result {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
        return Err(ApiError::new("NotSupportedError", error.to_string()));
    }
    Ok(())
}
fn reserve(active: &std::sync::atomic::AtomicUsize) -> web_api::Result<()> {
    use std::sync::atomic::Ordering;
    let mut count = active.load(Ordering::Relaxed);
    loop {
        if count >= 4 {
            return Err(ApiError::new(
                "QuotaExceededError",
                "desktop operation limit",
            ));
        }
        match active.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return Ok(()),
            Err(new) => count = new,
        }
    }
}
pub fn spawn_clipboard(
    text: Option<String>,
    finish: impl FnOnce(web_api::Result<String>) + Send + 'static,
) -> web_api::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    reserve(&ACTIVE)?;
    let result = std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    ACTIVE.fetch_sub(1, Ordering::Release);
                }
            }
            let _guard = Guard;
            finish(clipboard(text));
        });
    if let Err(error) = result {
        ACTIVE.fetch_sub(1, Ordering::Release);
        return Err(ApiError::new("NotSupportedError", error.to_string()));
    }
    Ok(())
}
fn run(command: &mut Command, input: Option<&[u8]>) -> web_api::Result<Vec<u8>> {
    if input.is_some_and(|s| s.len() > 1024 * 1024) {
        return Err(ApiError::new("QuotaExceededError", "clipboard quota"));
    }
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ApiError::new("NotSupportedError", e.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::new("NotSupportedError", "desktop pipe"))?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut out)
            .map(|_| out)
    });
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        if let Err(error) = stdin.write_all(input) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(ApiError::new("NotAllowedError", error.to_string()));
        }
    }
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| ApiError::new("NotAllowedError", e.to_string()))?
        {
            break status;
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(ApiError::new("TimeoutError", "desktop operation deadline"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = reader
        .join()
        .map_err(|_| ApiError::new("NotAllowedError", "desktop reader"))?
        .map_err(|e| ApiError::new("NotAllowedError", e.to_string()))?;
    if !status.success() {
        return Err(ApiError::new("NotAllowedError", "desktop operation failed"));
    }
    if output.len() > 1024 * 1024 {
        return Err(ApiError::new("QuotaExceededError", "clipboard quota"));
    }
    Ok(output)
}
