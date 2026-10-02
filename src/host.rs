//! The preview host: one process that owns every thumbnail, started on
//! demand by a capture and gone once its last thumbnail is.

use std::collections::BTreeSet;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use smithay_client_toolkit::data_device_manager::{WritePipe, data_source::DragSource};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::seat::pointer::{BTN_LEFT, PointerEvent, PointerEventKind};
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use wayland_client::protocol::{wl_data_device_manager::DndAction, wl_data_source::WlDataSource};
use wayland_client::{Connection, QueueHandle, protocol::wl_surface::WlSurface};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use crate::dnd::{self, Outcome};
use crate::ipc::{self, Reply, Request};
use crate::lock::Lock;
use crate::stack;
use crate::thumbnail::{Action, DragStart, Parts, Thumbnail};
use crate::wayland::{State, Wayland};

/// A host nobody connects to within this time gives up.
const STARTUP_GRACE: Duration = Duration::from_secs(2);

/// Connection bookkeeping and the exit rule, without any Wayland.
#[derive(Debug)]
pub struct Core {
    clients: BTreeSet<u64>,
    hiding: BTreeSet<u64>,
    seen_client: bool,
    started: Instant,
}

impl Core {
    pub fn new(now: Instant) -> Core {
        Core {
            clients: BTreeSet::new(),
            hiding: BTreeSet::new(),
            seen_client: false,
            started: now,
        }
    }

    pub fn connect(&mut self, id: u64) {
        self.clients.insert(id);
        self.seen_client = true;
    }

    /// Returns true if this request is what hides the thumbnails.
    pub fn hide(&mut self, id: u64) -> bool {
        let was_hidden = self.hidden();
        self.hiding.insert(id);
        !was_hidden
    }

    /// Returns true if this disconnect is what shows the thumbnails again.
    pub fn disconnect(&mut self, id: u64) -> bool {
        self.clients.remove(&id);
        self.hiding.remove(&id) && self.hiding.is_empty()
    }

    pub fn hidden(&self) -> bool {
        !self.hiding.is_empty()
    }

    /// Nothing to show and nobody talking to us. A fresh host waits a little
    /// for the capture that started it to connect.
    pub fn should_exit(&self, thumbnails: usize, now: Instant) -> bool {
        thumbnails == 0
            && self.clients.is_empty()
            && (self.seen_client || now.duration_since(self.started) > STARTUP_GRACE)
    }
}

pub struct Host {
    core: Core,
    /// Oldest first.
    thumbs: Vec<Thumbnail>,
    next_id: u64,
    timeout: Duration,
    conn: Connection,
    handle: LoopHandle<'static, State>,
    qh: QueueHandle<State>,
    drag: Option<Drag>,
    /// Threads still writing dropped data; the host waits for them, or the
    /// receiving app would get a cut-off file.
    writers: Arc<AtomicUsize>,
    warned_no_dnd: bool,
}

/// The drag-out in progress. Dropping the source cancels the drag.
struct Drag {
    thumb: u64,
    /// Kept here so the drop still delivers if the thumbnail goes away.
    path: PathBuf,
    source: DragSource,
    icon: WlSurface,
    icon_viewport: WpViewport,
}

/// Runs the host until its last thumbnail closes. A second host exits
/// right away: the first one owns the socket.
pub fn run() -> Result<()> {
    let Ok(_lock) = Lock::acquire(&ipc::host_lock_path()) else {
        tracing::info!("another preview host is running");
        return Ok(());
    };
    let socket = ipc::socket_path();
    // Safe under the lock: whatever is there is left over from a crash.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("could not listen on {}", socket.display()))?;
    listener.set_nonblocking(true)?;

    let config = crate::config::load(&crate::config::default_path())?;
    let Wayland {
        conn,
        queue,
        mut state,
        ..
    } = Wayland::connect()?;
    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("could not create the event loop")?;
    let handle = event_loop.handle();
    let qh = queue.handle();
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|e| anyhow::anyhow!("could not watch the Wayland socket: {e}"))?;

    state.preview = Some(Host {
        core: Core::new(Instant::now()),
        thumbs: Vec::new(),
        next_id: 0,
        timeout: Duration::from_secs(config.preview.timeout_secs),
        conn,
        handle: handle.clone(),
        qh,
        drag: None,
        writers: Arc::new(AtomicUsize::new(0)),
        warned_no_dnd: false,
    });

    let mut next_client = 0u64;
    handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            move |_, listener, state| {
                while let Ok((stream, _)) = listener.accept() {
                    next_client += 1;
                    if let Some(host) = &mut state.preview {
                        host.add_client(stream, next_client);
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("could not watch the socket: {e}"))?;

    tracing::info!("preview host listening on {}", socket.display());
    let signal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(500), &mut state, |state| {
            let host = state.preview.as_mut().expect("host state");
            host.tidy();
            let busy = host.thumbs.len()
                + host.writers.load(Ordering::SeqCst)
                + usize::from(host.drag.is_some());
            if host.core.should_exit(busy, Instant::now()) {
                signal.stop();
            }
        })
        .context("preview host event loop failed")?;

    let _ = std::fs::remove_file(&socket);
    tracing::info!("preview host done");
    Ok(())
}

impl Host {
    fn add_client(&mut self, stream: UnixStream, id: u64) {
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        self.core.connect(id);
        let mut buf = Vec::new();
        let inserted = self.handle.insert_source(
            Generic::new(stream, Interest::READ, Mode::Level),
            move |_, stream, state| {
                let mut chunk = [0u8; 4096];
                loop {
                    let mut reader: &UnixStream = stream;
                    match reader.read(&mut chunk) {
                        Ok(0) => {
                            disconnect(state, id);
                            return Ok(PostAction::Remove);
                        }
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => {
                            disconnect(state, id);
                            return Ok(PostAction::Remove);
                        }
                    }
                }
                while let Some(end) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=end).collect();
                    let reply = match ipc::parse_request(&String::from_utf8_lossy(&line)) {
                        Ok(request) => handle(state, id, request),
                        Err(e) => Reply::error(e),
                    };
                    let mut writer: &UnixStream = stream;
                    let _ = writer.write_all(ipc::encode(&reply).as_bytes());
                }
                Ok(PostAction::Continue)
            },
        );
        if inserted.is_err() {
            self.core.disconnect(id);
        }
    }

    /// Drops finished thumbnails and stacks the rest per output.
    fn tidy(&mut self) {
        let before = self.thumbs.len();
        self.thumbs.retain(|t| !t.is_finished());
        if self.thumbs.len() != before {
            self.restack();
            let _ = self.conn.flush();
        }
    }

    fn restack(&mut self) {
        let outputs: BTreeSet<String> = self.thumbs.iter().map(|t| t.output.clone()).collect();
        for output in outputs {
            let newest_first: Vec<usize> = (0..self.thumbs.len())
                .rev()
                .filter(|&i| self.thumbs[i].output == output)
                .collect();
            let heights: Vec<u32> = newest_first
                .iter()
                .map(|&i| self.thumbs[i].size.1)
                .collect();
            for (&i, margin) in newest_first.iter().zip(stack::bottom_margins(&heights)) {
                self.thumbs[i].set_margin(margin);
            }
        }
    }

    fn thumb(&mut self, surface: &WlSurface) -> Option<&mut Thumbnail> {
        self.thumbs.iter_mut().find(|t| t.is(surface))
    }

    pub fn configure(&mut self, layer: &LayerSurface, qh: &QueueHandle<State>) {
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.is_layer(layer)) {
            t.configure(qh);
        }
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        if let Some(t) = self.thumb(surface) {
            t.frame_done(qh);
        }
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        self.thumbs.retain(|t| !t.is_layer(layer));
    }

    /// Writes the dragged screenshot for the receiving app, on its own
    /// thread: a large PNG fills the pipe, and blocking here would freeze
    /// every thumbnail.
    pub fn send(&self, source: &WlDataSource, mime: String, fd: WritePipe) {
        let Some(drag) = self.drag.as_ref().filter(|d| d.source.inner() == source) else {
            return;
        };
        let path = drag.path.clone();
        let writers = Arc::clone(&self.writers);
        writers.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let mut fd = fd;
            if let Err(e) = dnd::write_offer(&mime, &path, &mut fd) {
                tracing::warn!("could not send the screenshot as {mime}: {e:#}");
            }
            drop(fd);
            writers.fetch_sub(1, Ordering::SeqCst);
        });
    }

    pub fn drag_ended(&mut self, source: &WlDataSource, outcome: Outcome, qh: &QueueHandle<State>) {
        let Some(drag) = self.drag.take_if(|d| d.source.inner() == source) else {
            return;
        };
        self.finish_drag(drag, outcome, qh);
    }

    fn finish_drag(&mut self, drag: Drag, outcome: Outcome, qh: &QueueHandle<State>) {
        tracing::info!("drag-out {outcome:?}");
        drag.icon_viewport.destroy();
        drag.icon.destroy();
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.id == drag.thumb) {
            t.end_drag(outcome, qh);
        }
        let _ = self.conn.flush();
    }

    fn expire(&mut self, id: u64) {
        let qh = self.qh.clone();
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.id == id) {
            t.expire(&qh);
        }
    }

    fn set_hidden(&mut self, hidden: bool) {
        let qh = self.qh.clone();
        for t in &mut self.thumbs {
            if hidden {
                t.hide(&qh);
            } else {
                t.show(&qh);
            }
        }
    }
}

/// Pointer events on thumbnails. A free function: starting a drag needs
/// more of `State` than the host.
pub fn pointer(state: &mut State, events: &[PointerEvent], qh: &QueueHandle<State>) {
    for event in events {
        let Some(host) = state.preview.as_mut() else {
            return;
        };
        let Some(i) = host.thumbs.iter().position(|t| t.is(&event.surface)) else {
            continue;
        };
        let (x, y) = event.position;
        match event.kind {
            PointerEventKind::Enter { serial } => {
                if let Some(cursor) = &state.cursor_device {
                    cursor.set_shape(serial, Shape::Pointer);
                }
            }
            PointerEventKind::Press {
                button: BTN_LEFT,
                serial,
                ..
            } => host.thumbs[i].press(x, y, serial),
            PointerEventKind::Motion { .. } => {
                let start = host.thumbs[i].motion(x, y, qh);
                if let Some(start) = start {
                    start_drag(state, i, start, qh);
                }
            }
            PointerEventKind::Release {
                button: BTN_LEFT, ..
            } => {
                let dragged = host.drag.as_ref().map(|d| d.thumb);
                if dnd::release_ends_drag(dragged, host.thumbs[i].id) {
                    tracing::info!("the compositor never took the drag over");
                    let drag = host.drag.take().expect("checked above");
                    host.finish_drag(drag, Outcome::Cancelled, qh);
                    continue;
                }
                let t = &mut host.thumbs[i];
                if t.release(x, y, qh) == Some(Action::OpenEditor) {
                    open_editor(&t.path);
                }
            }
            _ => {}
        }
    }
}

fn start_drag(state: &mut State, i: usize, start: DragStart, qh: &QueueHandle<State>) {
    let host = state.preview.as_mut().expect("host state");
    let (Some(manager), Some(device), Some(viewporter)) =
        (&state.data_devices, &state.data_device, &state.viewporter)
    else {
        if !host.warned_no_dnd {
            host.warned_no_dnd = true;
            tracing::warn!("the compositor has no wl_data_device_manager; drag-out is off");
        }
        return;
    };
    if host.drag.is_some() {
        return;
    }
    let source = manager.create_drag_and_drop_source(qh, dnd::MIME_TYPES, DndAction::Copy);
    let icon = state.compositor.create_surface(qh);
    let icon_viewport = viewporter.get_viewport(&icon, qh, ());
    let thumb = &mut host.thumbs[i];
    source.start_drag(device, thumb.surface(), Some(&icon), start.serial);
    thumb.draw_icon(&icon, &icon_viewport, start.grab);
    thumb.begin_drag(qh);
    tracing::info!("drag-out started for {}", thumb.path.display());
    host.drag = Some(Drag {
        thumb: thumb.id,
        path: thumb.path.clone(),
        source,
        icon,
        icon_viewport,
    });
    let _ = host.conn.flush();
}

fn disconnect(state: &mut State, id: u64) {
    let Some(host) = &mut state.preview else {
        return;
    };
    if host.core.disconnect(id) {
        host.set_hidden(false);
        let _ = host.conn.flush();
    }
}

fn handle(state: &mut State, id: u64, request: Request) -> Reply {
    match request {
        Request::Hide => {
            let host = state.preview.as_mut().expect("host state");
            if host.core.hide(id) {
                host.set_hidden(true);
                // The capture must not start until the compositor has
                // actually taken the thumbnails off the screen.
                if let Err(e) = host.conn.roundtrip() {
                    return Reply::error(format!("Wayland round trip failed: {e}"));
                }
            }
            Reply::ok()
        }
        Request::Add { path, output } => match add(state, &path, &output) {
            Ok(()) => Reply::ok(),
            Err(e) => {
                tracing::warn!("could not show a preview: {e:#}");
                Reply::error(format!("{e:#}"))
            }
        },
    }
}

fn add(state: &mut State, path: &Path, output_name: &str) -> Result<()> {
    let image = crate::thumbnail::load(path)?;
    let outputs = state.output_list();
    let output = outputs
        .iter()
        .find(|o| o.geom.name == output_name)
        .or_else(|| outputs.first())
        .context("the compositor reported no outputs")?;
    let parts = Parts {
        compositor: &state.compositor,
        layer_shell: state
            .layer_shell
            .as_ref()
            .context("compositor has no wlr-layer-shell")?,
        viewporter: state
            .viewporter
            .as_ref()
            .context("compositor has no wp-viewporter")?,
        shm: &state.shm,
    };
    let host = state.preview.as_mut().expect("host state");
    host.next_id += 1;
    let id = host.next_id;
    let mut thumb = Thumbnail::new(id, path.to_path_buf(), &image, output, &parts, &host.qh)?;
    if host.core.hidden() {
        thumb.hide(&host.qh);
    }
    host.thumbs.push(thumb);

    // Beyond MAX on this output, the oldest slide away.
    let qh = host.qh.clone();
    let open: Vec<usize> = (0..host.thumbs.len())
        .filter(|&i| host.thumbs[i].output == output.geom.name && !host.thumbs[i].is_closing())
        .collect();
    let dragging: Vec<bool> = open.iter().map(|&i| host.thumbs[i].is_dragging()).collect();
    for k in stack::evictions(&dragging) {
        host.thumbs[open[k]].slide_out(&qh);
    }
    host.restack();

    host.handle
        .insert_source(Timer::from_duration(host.timeout), move |_, _, state| {
            if let Some(host) = &mut state.preview {
                host.expire(id);
            }
            TimeoutAction::Drop
        })
        .map_err(|e| anyhow::anyhow!("could not start the preview timer: {e}"))?;
    let _ = host.conn.flush();
    Ok(())
}

/// Opens the screenshot in Satty; saving there overwrites the file.
fn open_editor(path: &Path) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut command = Command::new("satty");
    command
        .arg("--filename")
        .arg(path)
        .arg("--output-filename")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
    }
    if let Err(e) = command.spawn() {
        tracing::warn!("could not start satty: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_until_every_hider_leaves() {
        let mut core = Core::new(Instant::now());
        core.connect(1);
        core.connect(2);
        assert!(core.hide(1), "first hide hides");
        assert!(!core.hide(2), "second hide changes nothing");
        assert!(!core.disconnect(1), "still hidden by 2");
        assert!(core.hidden());
        assert!(core.disconnect(2), "last hider leaving shows");
        assert!(!core.hidden());
    }

    #[test]
    fn a_client_that_never_hid_does_not_show() {
        let mut core = Core::new(Instant::now());
        core.connect(1);
        core.connect(2);
        core.hide(1);
        assert!(!core.disconnect(2));
        assert!(core.hidden());
    }

    #[test]
    fn exits_only_when_empty_and_alone() {
        let now = Instant::now();
        let mut core = Core::new(now);
        assert!(!core.should_exit(0, now), "waits for its first client");
        assert!(
            core.should_exit(0, now + Duration::from_secs(3)),
            "gives up after the grace period"
        );

        core.connect(1);
        assert!(!core.should_exit(0, now), "a client is connected");
        core.disconnect(1);
        assert!(!core.should_exit(1, now), "a thumbnail is up");
        assert!(core.should_exit(0, now));
    }

    #[test]
    fn a_second_host_gives_up_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw-preview.lock");
        let _first = Lock::acquire(&path).unwrap();
        assert!(Lock::acquire(&path).is_err());
    }
}
