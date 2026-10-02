use std::time::Instant;

use anyhow::{Context, Result, bail};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    data_device_manager::{
        DataDeviceManagerState, WritePipe,
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::DataSourceHandler,
    },
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerHandler, cursor_shape::CursorShapeManager},
    },
    shell::wlr_layer::{LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::{GlobalList, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer, wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
        wl_data_source::WlDataSource, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface,
    },
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1;
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::capture::Pending;
use crate::dnd::Outcome;
use crate::error::HintExt;
use crate::frame::OutputGeom;
use crate::host::Host;
use crate::region::Overlay;

/// An output as valw sees it.
#[derive(Debug, Clone)]
pub struct Output {
    pub wl: wl_output::WlOutput,
    pub geom: OutputGeom,
    pub transform: wl_output::Transform,
    /// Physical pixels per logical pixel, e.g. 1.25.
    pub scale: f64,
}

pub struct Wayland {
    pub conn: Connection,
    pub queue: EventQueue<State>,
    pub globals: GlobalList,
    pub state: State,
}

pub struct State {
    registry: RegistryState,
    pub outputs: OutputState,
    pub seat: SeatState,
    pub shm: Shm,
    pub compositor: CompositorState,
    pub layer_shell: Option<LayerShell>,
    pub screencopy: Option<ZwlrScreencopyManagerV1>,
    pub viewporter: Option<WpViewporter>,
    pub cursor_shape: Option<CursorShapeManager>,
    pub cursor_device: Option<WpCursorShapeDeviceV1>,
    /// For dragging a preview out; the host never receives drops.
    pub data_devices: Option<DataDeviceManagerState>,
    pub data_device: Option<DataDevice>,
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub pointer: Option<wl_pointer::WlPointer>,
    /// Screencopy requests in flight.
    pub captures: Vec<Pending>,
    /// The region selection overlay, while it is open.
    pub overlay: Option<Overlay>,
    /// The preview thumbnails, in the preview host process.
    pub preview: Option<Host>,
}

impl Wayland {
    pub fn connect() -> Result<Wayland> {
        let conn = Connection::connect_to_env()
            .context("could not connect to the Wayland compositor")
            .hint("valw needs a Wayland session; is WAYLAND_DISPLAY set?")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)
            .context("could not read the compositor's globals")?;
        let qh = queue.handle();

        let mut state = State {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            seat: SeatState::new(&globals, &qh),
            shm: Shm::bind(&globals, &qh).context("compositor has no wl_shm")?,
            compositor: CompositorState::bind(&globals, &qh)
                .context("compositor has no wl_compositor")?,
            layer_shell: LayerShell::bind(&globals, &qh).ok(),
            screencopy: globals.bind(&qh, 1..=3, ()).ok(),
            viewporter: globals.bind(&qh, 1..=1, ()).ok(),
            cursor_shape: CursorShapeManager::bind(&globals, &qh).ok(),
            cursor_device: None,
            data_devices: DataDeviceManagerState::bind(&globals, &qh).ok(),
            data_device: None,
            keyboard: None,
            pointer: None,
            captures: Vec::new(),
            overlay: None,
            preview: None,
        };
        // Two round trips: one for wl_output, one for the xdg-output details.
        queue
            .roundtrip(&mut state)
            .context("Wayland round trip failed")?;
        queue
            .roundtrip(&mut state)
            .context("Wayland round trip failed")?;

        let wl = Wayland {
            conn,
            queue,
            globals,
            state,
        };
        wl.log_environment();
        Ok(wl)
    }

    /// All outputs with a known name and logical geometry.
    pub fn outputs(&self) -> Vec<Output> {
        self.state.output_list()
    }

    /// `(interface, version)` of every global, sorted.
    pub fn global_list(&self) -> Vec<(String, u32)> {
        let mut list: Vec<_> = self
            .globals
            .contents()
            .clone_list()
            .into_iter()
            .map(|g| (g.interface, g.version))
            .collect();
        list.sort();
        list.dedup();
        list
    }

    fn log_environment(&self) {
        let globals: Vec<String> = self
            .global_list()
            .into_iter()
            .map(|(interface, version)| format!("{interface} v{version}"))
            .collect();
        tracing::info!("globals: {}", globals.join(", "));
        for o in self.outputs() {
            let g = &o.geom;
            tracing::info!(
                "output {} at {},{} size {}x{} (logical) transform {:?}",
                g.name,
                g.x,
                g.y,
                g.width,
                g.height,
                o.transform
            );
        }
    }

    /// Dispatches events until `done` returns true or `deadline` passes.
    pub fn dispatch_until(
        &mut self,
        deadline: Instant,
        mut done: impl FnMut(&State) -> bool,
    ) -> Result<()> {
        loop {
            self.queue
                .dispatch_pending(&mut self.state)
                .context("Wayland dispatch failed")?;
            if done(&self.state) {
                return Ok(());
            }
            self.queue.flush().context("Wayland flush failed")?;
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                bail!("the compositor did not answer in time");
            }
            let timeout = Timespec::try_from(left).expect("duration fits");
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) => continue,
                Ok(_) => {
                    guard.read().context("Wayland read failed")?;
                }
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(e).context("poll failed"),
            }
        }
    }

    /// Dispatches events until `done` returns true. Waits as long as it takes.
    pub fn dispatch_blocking(&mut self, mut done: impl FnMut(&State) -> bool) -> Result<()> {
        while !done(&self.state) {
            self.queue
                .blocking_dispatch(&mut self.state)
                .context("Wayland dispatch failed")?;
        }
        Ok(())
    }
}

impl State {
    /// All outputs with a known name and logical geometry.
    pub fn output_list(&self) -> Vec<Output> {
        self.outputs
            .outputs()
            .filter_map(|wl| {
                let info = self.outputs.info(&wl)?;
                let (x, y) = info.logical_position?;
                let (width, height) = info.logical_size?;
                Some(Output {
                    scale: output_scale(&info, width),
                    geom: OutputGeom {
                        name: info
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("output-{}", info.id)),
                        x,
                        y,
                        width,
                        height,
                    },
                    transform: info.transform,
                    wl,
                })
            })
            .collect()
    }
}

/// Physical pixels per logical pixel: the current mode's width (turned to
/// match the logical orientation) over the logical width.
fn output_scale(info: &smithay_client_toolkit::output::OutputInfo, logical_width: i32) -> f64 {
    let Some(mode) = info.modes.iter().find(|m| m.current) else {
        return info.scale_factor as f64;
    };
    let (w, h) = mode.dimensions;
    let rotated = matches!(
        info.transform,
        wl_output::Transform::_90
            | wl_output::Transform::_270
            | wl_output::Transform::Flipped90
            | wl_output::Transform::Flipped270
    );
    let physical_width = if rotated { h } else { w };
    if logical_width > 0 {
        physical_width as f64 / logical_width as f64
    } else {
        1.0
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(overlay) = &mut self.overlay {
            overlay.frame_done(surface, qh);
        }
        if let Some(preview) = &mut self.preview {
            preview.frame_done(surface, qh);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(overlay) = &mut self.overlay {
            overlay.cancel();
        }
        if let Some(preview) = &mut self.preview {
            preview.closed(layer);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(overlay) = &mut self.overlay {
            overlay.configure(layer, configure.new_size, qh);
        }
        if let Some(preview) = &mut self.preview {
            preview.configure(layer, qh);
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer
            && self.pointer.is_none()
            && let Ok(pointer) = self.seat.get_pointer(qh, &seat)
        {
            self.cursor_device = self
                .cursor_shape
                .as_ref()
                .map(|m| m.get_shape_device(&pointer, qh));
            self.data_device = self
                .data_devices
                .as_ref()
                .map(|m| m.get_data_device(qh, &seat));
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
        if capability == Capability::Pointer {
            if let Some(d) = self.cursor_device.take() {
                d.destroy();
            }
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        let Some(overlay) = &mut self.overlay else {
            return;
        };
        match event.keysym {
            Keysym::Escape => overlay.cancel(),
            Keysym::space => overlay.switch_to_window(),
            _ => {}
        }
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        if let Some(overlay) = &mut self.overlay {
            overlay.pointer(events, self.cursor_device.as_ref(), qh);
        }
        if self.preview.is_some() {
            crate::host::pointer(self, events, qh);
        }
    }
}

impl DataSourceHandler for State {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        fd: WritePipe,
    ) {
        if let Some(preview) = &self.preview {
            preview.send(source, mime, fd);
        }
    }

    fn cancelled(&mut self, _: &Connection, qh: &QueueHandle<Self>, source: &WlDataSource) {
        if let Some(preview) = &mut self.preview {
            preview.drag_ended(source, Outcome::Cancelled, qh);
        }
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, qh: &QueueHandle<Self>, source: &WlDataSource) {
        if let Some(preview) = &mut self.preview {
            preview.drag_ended(source, Outcome::Dropped, qh);
        }
    }

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

// valw only ever starts drags; incoming offers are ignored.
impl DataDeviceHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &wl_surface::WlSurface,
    ) {
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for State {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
smithay_client_toolkit::delegate_dispatch2!(State);
wayland_client::delegate_noop!(State: ZwlrScreencopyManagerV1);
wayland_client::delegate_noop!(State: WpViewporter);
wayland_client::delegate_noop!(State: WpViewport);
wayland_client::delegate_noop!(State: ignore WlBuffer);
