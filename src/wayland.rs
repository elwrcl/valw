use std::time::Instant;

use anyhow::{Context, Result, bail};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
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
    protocol::{wl_buffer::WlBuffer, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1;
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::capture::Pending;
use crate::error::HintExt;
use crate::frame::OutputGeom;

/// An output as valw sees it.
#[derive(Debug, Clone)]
pub struct Output {
    pub wl: wl_output::WlOutput,
    pub geom: OutputGeom,
    pub transform: wl_output::Transform,
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
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub pointer: Option<wl_pointer::WlPointer>,
    /// Screencopy requests in flight.
    pub captures: Vec<Pending>,
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
            keyboard: None,
            pointer: None,
            captures: Vec::new(),
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
        self.state
            .outputs
            .outputs()
            .filter_map(|wl| {
                let info = self.state.outputs.info(&wl)?;
                let (x, y) = info.logical_position?;
                let (width, height) = info.logical_size?;
                Some(Output {
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

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

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
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {}

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
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
        _: KeyEvent,
    ) {
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
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        _: &[PointerEvent],
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
