//! State shared between the UI thread and a session's I/O backend.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use tokio::sync::oneshot;

use super::palette;

pub type TermHandle = Arc<FairMutex<Term<Listener>>>;

/// Byte sink of a session (local PTY or SSH channel).
pub trait PtyIo: Send + Sync {
    fn write(&self, data: Cow<'static, [u8]>);
    fn resize(&self, size: WindowSize);
    fn shutdown(&self);
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    Running,
    Exited(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PromptKind {
    Secret,
    Text,
    Confirm,
}

/// A question from a backend to the user (password, host key, ...).
pub struct Prompt {
    pub title: String,
    pub message: String,
    pub kind: PromptKind,
    pub reply: oneshot::Sender<Option<String>>,
}

pub struct Shared {
    ctx: egui::Context,
    title: Mutex<String>,
    status: Mutex<Status>,
    size: Mutex<WindowSize>,
    io: OnceLock<Box<dyn PtyIo>>,
    prompts: Mutex<VecDeque<Prompt>>,
}

impl Shared {
    pub fn new(ctx: egui::Context, title: String, size: WindowSize) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            title: Mutex::new(title),
            status: Mutex::new(Status::Connecting),
            size: Mutex::new(size),
            io: OnceLock::new(),
            prompts: Mutex::new(VecDeque::new()),
        })
    }

    pub fn repaint(&self) {
        self.ctx.request_repaint();
    }

    pub fn title(&self) -> String {
        self.title.lock().unwrap().clone()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn set_status(&self, status: Status) {
        let mut cur = self.status.lock().unwrap();
        // The first exit reason is the most specific one.
        if !matches!(*cur, Status::Exited(_)) {
            *cur = status;
        }
        drop(cur);
        self.repaint();
    }

    pub fn size(&self) -> WindowSize {
        *self.size.lock().unwrap()
    }

    pub fn set_io(&self, io: Box<dyn PtyIo>) {
        let _ = self.io.set(io);
    }

    pub fn write(&self, data: Cow<'static, [u8]>) {
        if let Some(io) = self.io.get() {
            io.write(data);
        }
    }

    pub fn resize(&self, size: WindowSize) {
        *self.size.lock().unwrap() = size;
        if let Some(io) = self.io.get() {
            io.resize(size);
        }
    }

    pub fn shutdown(&self) {
        if let Some(io) = self.io.get() {
            io.shutdown();
        }
    }

    /// Asks the user and waits for the answer. `None` means cancelled.
    pub async fn ask(&self, title: String, message: String, kind: PromptKind) -> Option<String> {
        let (reply, rx) = oneshot::channel();
        self.prompts.lock().unwrap().push_back(Prompt { title, message, kind, reply });
        self.repaint();
        rx.await.ok().flatten()
    }

    pub fn take_prompt(&self) -> Option<Prompt> {
        self.prompts.lock().unwrap().pop_front()
    }
}

/// Terminal grid dimensions.
#[derive(Clone, Copy)]
pub struct GridSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Receives events emitted by the terminal emulator.
#[derive(Clone)]
pub struct Listener(pub Arc<Shared>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let shared = &self.0;
        match event {
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => shared.repaint(),
            Event::Title(title) => {
                *shared.title.lock().unwrap() = title;
                shared.repaint();
            }
            Event::ResetTitle => {}
            Event::PtyWrite(text) => shared.write(Cow::Owned(text.into_bytes())),
            Event::ClipboardStore(_, text) => shared.ctx.copy_text(text),
            Event::ColorRequest(index, format) => {
                shared.write(Cow::Owned(format(palette::default_color(index)).into_bytes()))
            }
            Event::TextAreaSizeRequest(format) => shared.write(Cow::Owned(format(shared.size()).into_bytes())),
            Event::ChildExit(_) | Event::Exit => shared.set_status(Status::Exited(String::new())),
            Event::ClipboardLoad(..) | Event::Bell => {}
        }
    }
}
