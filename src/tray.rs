//! Notification area icon. Closing the window hides it while this is alive.

use std::collections::VecDeque;
use std::sync::Mutex;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const SHOW: &str = "show";
const QUIT: &str = "quit";

static ACTIONS: Mutex<VecDeque<Action>> = Mutex::new(VecDeque::new());

enum Action {
    Show,
    Quit,
}

pub enum Event {
    Show,
    Quit,
}

pub struct Resident {
    icon: TrayIcon,
}

pub fn bind(ctx: &egui::Context) {
    let wake = ctx.clone();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let show = matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            }
        );
        if show {
            push(Action::Show);
        }
        wake.request_repaint();
    }));
    let wake = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id.as_ref() == SHOW {
            push(Action::Show);
        } else if event.id.as_ref() == QUIT {
            push(Action::Quit);
        }
        wake.request_repaint();
    }));
}

pub fn install(rgba: &[u8], width: u32, height: u32) -> Result<Resident, String> {
    let icon = tray_icon::Icon::from_rgba(rgba.to_vec(), width, height)
        .map_err(|error| format!("タスクトレイのアイコンを作れません: {error}"))?;
    let menu = Menu::new();
    let show = MenuItem::with_id(SHOW, "表示", true, None);
    let quit = MenuItem::with_id(QUIT, "終了", true, None);
    menu.append_items(&[&show, &quit])
        .map_err(|error| format!("タスクトレイのメニューを作れません: {error}"))?;
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("IkTerminal")
        .with_icon(icon)
        .with_menu_on_left_click(false)
        .build()
        .map_err(|error| format!("タスクトレイに入れられません: {error}"))?;
    Ok(Resident { icon })
}

impl Resident {
    pub fn set_tooltip(&self, text: &str) {
        let _ = self.icon.set_tooltip(Some(text));
    }
}

pub fn poll() -> Option<Event> {
    let action = ACTIONS.lock().ok()?.pop_front()?;
    Some(match action {
        Action::Show => Event::Show,
        Action::Quit => Event::Quit,
    })
}

fn push(action: Action) {
    if let Ok(mut actions) = ACTIONS.lock() {
        actions.push_back(action);
    }
}
