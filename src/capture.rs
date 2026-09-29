use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use smithay_client_toolkit::shm::raw::RawPool;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{wl_buffer::WlBuffer, wl_shm::Format},
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self, ZwlrScreencopyFrameV1,
};

use crate::error::HintExt;
use crate::frame::{self, Frame, SUPPORTED};
use crate::wayland::{Output, State, Wayland};

/// How long the compositor gets to deliver all frames.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Something that captures outputs. Only wlr-screencopy exists today;
/// ext-image-copy-capture would be a second implementation.
pub trait Capturer {
    fn capture(&mut self, outputs: &[Output], cursor: bool) -> Result<Vec<Frame>>;
}

/// A buffer layout the compositor offered.
#[derive(Debug, Clone, Copy)]
struct Layout {
    format: Format,
    width: u32,
    height: u32,
    stride: u32,
}

/// One screencopy request in flight.
pub struct Pending {
    output: Output,
    offered: Vec<Format>,
    layout: Option<Layout>,
    // niri rejects a screencopy buffer unless its pool is exactly the
    // buffer's size, so every capture gets a pool of its own.
    pool: Option<RawPool>,
    buffer: Option<WlBuffer>,
    y_invert: bool,
    result: Option<Result<Frame>>,
}

impl Capturer for Wayland {
    fn capture(&mut self, outputs: &[Output], cursor: bool) -> Result<Vec<Frame>> {
        let manager = self
            .state
            .screencopy
            .clone()
            .context("compositor does not support wlr-screencopy")
            .hint("run `valw doctor` to see what the compositor supports")?;
        let qh = self.queue.handle();

        self.state.captures = outputs
            .iter()
            .map(|o| Pending {
                output: o.clone(),
                offered: Vec::new(),
                layout: None,
                pool: None,
                buffer: None,
                y_invert: false,
                result: None,
            })
            .collect();
        for (i, o) in outputs.iter().enumerate() {
            manager.capture_output(cursor as i32, &o.wl, &qh, i);
        }

        let started = Instant::now();
        self.dispatch_until(started + TIMEOUT, |s| {
            s.captures.iter().all(|c| c.result.is_some())
        })
        .context("screen capture failed")?;
        tracing::info!(
            "captured {} output(s) in {:?} ({:?} after start)",
            outputs.len(),
            started.elapsed(),
            crate::log::since_start()
        );

        std::mem::take(&mut self.state.captures)
            .into_iter()
            .map(|c| {
                let name = c.output.geom.name.clone();
                c.result
                    .expect("checked above")
                    .with_context(|| format!("could not capture output {name}"))
            })
            .collect()
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, usize> for State {
    fn event(
        state: &mut Self,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        &i: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event;
        match event {
            Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                let c = &mut state.captures[i];
                c.offered.push(format);
                let better = c.layout.is_none_or(|l| rank(format) < rank(l.format));
                if SUPPORTED.contains(&format) && better {
                    c.layout = Some(Layout {
                        format,
                        width,
                        height,
                        stride,
                    });
                }
                // Before v3 there is no buffer_done; the first buffer event is all we get.
                if frame.version() < 3 {
                    start_copy(state, frame, i, qh);
                }
            }
            Event::BufferDone => start_copy(state, frame, i, qh),
            Event::Flags { flags } => {
                state.captures[i].y_invert = flags
                    .into_result()
                    .is_ok_and(|f| f.contains(zwlr_screencopy_frame_v1::Flags::YInvert));
            }
            Event::Ready { .. } => {
                let c = &mut state.captures[i];
                c.result = Some(finish(c));
                if let Some(buffer) = c.buffer.take() {
                    buffer.destroy();
                }
                frame.destroy();
            }
            Event::Failed => {
                state.captures[i].result = Some(Err(anyhow!("the compositor refused the capture")));
                frame.destroy();
            }
            _ => {}
        }
    }
}

fn rank(format: Format) -> usize {
    SUPPORTED
        .iter()
        .position(|&f| f == format)
        .unwrap_or(usize::MAX)
}

fn start_copy(state: &mut State, frame: &ZwlrScreencopyFrameV1, i: usize, qh: &QueueHandle<State>) {
    let c = &mut state.captures[i];
    if c.buffer.is_some() || c.result.is_some() {
        return;
    }
    let Some(l) = c.layout else {
        c.result = Some(
            Err(anyhow!(
                "compositor offered only {:?}, which valw can't convert yet",
                c.offered
            ))
            .hint("run `valw doctor` and include its output in a bug report"),
        );
        frame.destroy();
        return;
    };
    tracing::info!(
        "{}: offered {:?}, using {:?} {}x{}",
        c.output.geom.name,
        c.offered,
        l.format,
        l.width,
        l.height
    );
    let len = (l.stride * l.height) as usize;
    match RawPool::new(len, &state.shm) {
        Ok(mut pool) => {
            let buffer = pool.create_buffer(
                0,
                l.width as i32,
                l.height as i32,
                l.stride as i32,
                l.format,
                (),
                qh,
            );
            frame.copy(&buffer);
            c.pool = Some(pool);
            c.buffer = Some(buffer);
        }
        Err(e) => {
            c.result = Some(Err(e).context("could not allocate a capture buffer"));
            frame.destroy();
        }
    }
}

fn finish(c: &mut Pending) -> Result<Frame> {
    let l = c.layout.context("ready without a buffer")?;
    let pool = c.pool.as_mut().context("ready without a buffer")?;
    let data = &pool.mmap()[..];
    let pixels = frame::to_bgrx(l.format, l.width, l.height, l.stride, data, c.y_invert)?;
    Ok(Frame {
        pixels: Arc::new(frame::apply_transform(pixels, c.output.transform)),
        output: c.output.geom.clone(),
    })
}
