//! MP-08/MP-10/MP-11: bounded native exact preparation, outside input/capture.
//! All codec certificates are computed on the worker thread before submission.
use super::{
    raster,
    worker::{epoch, reply},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread::JoinHandle;

pub(super) struct ExactPlan {
    pub pixels: Vec<u8>,
    pub rectangles: Vec<[i32; 4]>,
    pub w: i32,
    pub h: i32,
    pub patch: bool,
    pub revision: Option<u64>,
    pub started: f64,
}
impl ExactPlan {
    fn finish(self) -> Result<Value, String> {
        let mut value = json!({"width":self.w,"height":self.h,"native_exact":true});
        if !self.patch {
            value["data_base64"] = STANDARD
                .encode(raster::png(
                    &self.pixels,
                    self.w as u32,
                    self.h as u32,
                    self.w as usize * 4,
                )?)
                .into();
        }
        let mut tiles = Vec::new();
        for [x, y, right, bottom] in self.rectangles {
            let (width, height) = (right - x, bottom - y);
            let bytes = raster::png(
                &self.pixels[((y * self.w + x) * 4) as usize..],
                width as u32,
                height as u32,
                self.w as usize * 4,
            )?;
            tiles.push(json!({"x":x,"y":y,"width":width,"height":height,"data_base64":STANDARD.encode(bytes)}));
        }
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
pub(super) struct ExactWorker {
    sender: Option<SyncSender<(u64, ExactPlan)>>,
    thread: Option<JoinHandle<()>>,
}
impl ExactWorker {
    pub fn new() -> Self {
        let (sender, receiver) = sync_channel::<(u64, ExactPlan)>(8);
        let thread = std::thread::spawn(move || {
            while let Ok((id, plan)) = receiver.recv() {
                // stdout's global lock keeps each reply/frame indivisible.
                let value = plan
                    .finish()
                    .unwrap_or_else(|_| json!({"error":"MP-11: exact preparation failed"}));
                if reply(id, value).is_err() {
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
        self.sender
            .as_ref()
            .unwrap()
            .try_send((id, plan))
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
