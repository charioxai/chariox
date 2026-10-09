//! MP-08/MP-10/MP-11: bounded native exact preparation, outside input/capture.
//! All codec certificates are computed on the worker thread before submission.
use super::{
    raster,
    worker::{epoch, exact_reply},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread::JoinHandle;

pub(super) struct ExactPlan {
    /// Masked full raster (repairs); empty for patches, which carry `tiles`.
    pub pixels: Vec<u8>,
    pub tiles: Vec<([i32; 4], Vec<u8>)>,
    pub rectangles: Vec<[i32; 4]>,
    pub w: i32,
    pub h: i32,
    pub patch: bool,
    pub repair_only: bool,
    pub revision: Option<u64>,
    pub started: f64,
}
impl ExactPlan {
    pub(super) fn finish(self) -> Result<Value, String> {
        let mut value = json!({"width":self.w,"height":self.h,"native_exact":true});
        if self.repair_only && !self.patch {
            value["native_repair"] = true.into();
        }
        if !self.patch && !self.repair_only {
            value["data_base64"] = STANDARD
                .encode(raster::png(
                    &self.pixels,
                    self.w as u32,
                    self.h as u32,
                    self.w as usize * 4,
                )?)
                .into();
        }
        // MP-08/MP-10: small input patches stay PNG for latency. Repairs merge
        // adjacent tiles of each row band into lossless WebP strips, about
        // half the bytes of tile PNG on text. Higher effort also reduces
        // paced transfer on dense1080p text, as it does at Retina density.
        // Up to eight threads, in order.
        let (pixels, w, patch) = (&self.pixels, self.w, self.patch);
        let rectangles = if patch {
            self.rectangles.clone()
        } else {
            strips(&self.rectangles)
        };
        let tiles = &self.tiles;
        let encode = |&[x, y, right, bottom]: &[i32; 4]| -> Result<Value, String> {
            let (width, height) = (right - x, bottom - y);
            let (format, bytes) = if patch {
                let (_, tile) = tiles
                    .iter()
                    .find(|(r, _)| *r == [x, y, width, height])
                    .ok_or("MP-11: patch tile")?;
                (
                    "png",
                    raster::png(tile, width as u32, height as u32, width as usize * 4)?,
                )
            } else {
                (
                    "webp",
                    raster::webp(pixels, w as usize * 4, [x, y, width, height], 1, 50)?,
                )
            };
            Ok(
                json!({"x":x,"y":y,"width":width,"height":height,"format":format,"data_base64":STANDARD.encode(bytes)}),
            )
        };
        let part = rectangles.len().div_ceil(8).max(1);
        let tiles = std::thread::scope(|scope| {
            let workers = rectangles
                .chunks(part)
                .map(|rects| {
                    scope.spawn(move || rects.iter().map(encode).collect::<Result<Vec<_>, _>>())
                })
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .try_fold(Vec::new(), |mut tiles, worker| {
                    tiles.extend(worker.join().map_err(|_| "MP-10: exact PNG worker")??);
                    Ok::<_, String>(tiles)
                })
        })?;
        value[if self.patch {
            "native_tiles"
        } else {
            "repair_tiles"
        }] = tiles.into();
        if let Some(revision) = self.revision {
            value["native_revision"] = revision.into();
        }
        value["timings"] = json!([["native_exact_prepare", self.started, epoch()]]);
        Ok(value)
    }
}
/// Merge each 128-row band's neighbouring repair clips ([l,t,r,b]) into one
/// strip; uncovered gaps are below a tile and are encoded exactly as well.
fn strips(rectangles: &[[i32; 4]]) -> Vec<[i32; 4]> {
    let mut sorted = rectangles.to_vec();
    sorted.sort_by_key(|r| (r[1] / 128, r[0]));
    let mut output: Vec<[i32; 4]> = Vec::new();
    for r in sorted {
        match output.last_mut() {
            Some(last) if last[1] / 128 == r[1] / 128 && r[0] <= last[2] + 128 => {
                last[1] = last[1].min(r[1]);
                last[2] = last[2].max(r[2]);
                last[3] = last[3].max(r[3]);
            }
            _ => output.push(r),
        }
    }
    output
}
/// MP-08/MP-10: encode work that replies off the capture thread.
pub(super) type Job = Box<dyn FnOnce() -> Value + Send>;
pub(super) struct ExactWorker {
    sender: Option<SyncSender<(u64, Job)>>,
    thread: Option<JoinHandle<()>>,
}
impl ExactWorker {
    pub fn new() -> Self {
        let (sender, receiver) = sync_channel::<(u64, Job)>(8);
        let thread = std::thread::spawn(move || {
            while let Ok((id, job)) = receiver.recv() {
                // stdout's global lock keeps each reply/frame indivisible.
                if exact_reply(id, job()).is_err() {
                    break;
                }
            }
        });
        Self {
            sender: Some(sender),
            thread: Some(thread),
        }
    }
    pub fn submit(&self, id: u64, plan: ExactPlan) -> Result<(), String> {
        self.submit_job(
            id,
            Box::new(move || {
                plan.finish()
                    .unwrap_or_else(|_| json!({"error":"MP-11: exact preparation failed"}))
            }),
        )
    }
    pub fn submit_job(&self, id: u64, job: Job) -> Result<(), String> {
        self.sender
            .as_ref()
            .unwrap()
            .try_send((id, job))
            .map_err(|_| "MP-11: native exact queue unavailable".into())
    }
}
impl Drop for ExactWorker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
