//! Serial port (COM) session. Bytes are passed through to the terminal as-is.

use std::borrow::Cow;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::vte::ansi::Processor;

use crate::terminal::{PtyIo, Shared, Status, TermHandle};

const BAUD_RATES: [u32; 8] = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

pub fn baud_rates() -> &'static [u32] {
    &BAUD_RATES
}

pub fn list_ports() -> Vec<String> {
    let mut names: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|port| port.port_name)
        .collect();
    names.sort_by(|a, b| com_key(a).cmp(&com_key(b)));
    names
}

fn com_key(name: &str) -> (u32, &str) {
    let number = name
        .strip_prefix("COM")
        .and_then(|rest| rest.parse().ok())
        .unwrap_or(u32::MAX);
    (number, name)
}

enum Command {
    Input(Vec<u8>),
    Shutdown,
}

struct SerialIo(mpsc::Sender<Command>);

impl PtyIo for SerialIo {
    fn write(&self, data: Cow<'static, [u8]>) {
        if !data.is_empty() {
            let _ = self.0.send(Command::Input(data.into_owned()));
        }
    }

    fn resize(&self, _size: WindowSize) {}

    fn shutdown(&self) {
        let _ = self.0.send(Command::Shutdown);
    }
}

/// Opens `port` at `baud` (8N1, no flow control) and pumps bytes on a background thread.
pub fn spawn(
    shared: std::sync::Arc<Shared>,
    term: TermHandle,
    port: &str,
    baud: u32,
) -> std::io::Result<()> {
    let mut opened = serialport::new(port, baud)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(50))
        .open()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let (tx, rx) = mpsc::channel();
    shared.set_io(Box::new(SerialIo(tx)));
    shared.set_status(Status::Running);
    let label = port.to_owned();
    std::thread::Builder::new()
        .name("ikterm-serial".to_owned())
        .spawn(move || run(&shared, &term, &mut *opened, rx, &label))
        .map(|_| ())
}

fn run(
    shared: &Shared,
    term: &TermHandle,
    port: &mut dyn serialport::SerialPort,
    rx: mpsc::Receiver<Command>,
    label: &str,
) {
    let mut parser: Processor = Processor::new();
    let mut buf = [0_u8; 4096];
    let mut closed_by_user = false;
    let mut error = None;
    loop {
        if let Some(deadline) = parser.sync_timeout().sync_timeout()
            && Instant::now() >= deadline
        {
            parser.stop_sync(&mut *term.lock());
            shared.repaint();
        }
        loop {
            match rx.try_recv() {
                Ok(Command::Input(bytes)) => {
                    if let Err(e) = port.write_all(&bytes) {
                        error = Some(e.to_string());
                        break;
                    }
                }
                Ok(Command::Shutdown) => {
                    closed_by_user = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    closed_by_user = true;
                    break;
                }
            }
        }
        if closed_by_user || error.is_some() {
            break;
        }
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                parser.advance(&mut *term.lock(), &buf[..n]);
                shared.repaint();
            }
            Err(e) if is_timeout(&e) => {}
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        }
    }
    if !closed_by_user {
        let message = match error {
            Some(e) => format!("{label} が切断されました: {e}"),
            None => format!("{label} が切断されました"),
        };
        shared.set_status(Status::Exited(message));
    }
}

fn is_timeout(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::TimedOut
}
