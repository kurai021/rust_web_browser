//! CPU layout/glyph preparation is isolated from window input and presentation.
//! Coalesce queued resize/content revisions; UI rejects obsolete results.
use crate::page::Page;
use cosmic_text::FontSystem;
use layout::{LayoutResult, Size};
use paint::{Rasterizer, Scene};
use std::sync::{mpsc, Arc};

pub struct Request {
    pub navigation: u64,
    pub revision: u64,
    pub viewport: Size,
    pub page: Arc<Page>,
    pub styles: css::ComputedStyles,
    pub font_generation: u64,
    pub locale: String,
    pub db: cosmic_text::fontdb::Database,
}
pub struct Ready {
    pub navigation: u64,
    pub revision: u64,
    pub viewport: Size,
    pub layout: LayoutResult,
    pub scene: Scene,
    pub elapsed_ms: f64,
    pub passes: u64,
}
pub struct LayoutWorker {
    commands: Option<mpsc::Sender<Request>>,
    results: mpsc::Receiver<Ready>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl LayoutWorker {
    pub fn spawn(wake: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Request>();
        let (done, results) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("layout-worker".into())
            .spawn(move || {
                let mut fonts = None;
                let mut generation = None;
                let mut rasterizer = Rasterizer::default();
                let mut passes = 0;
                while let Ok(mut request) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        request = newer;
                    }
                    if generation != Some(request.font_generation) {
                        fonts = Some(FontSystem::new_with_locale_and_db(
                            request.locale,
                            request.db,
                        ));
                        generation = Some(request.font_generation);
                        rasterizer.clear();
                    }
                    let Some(fonts) = &mut fonts else {
                        continue;
                    };
                    let started = std::time::Instant::now();
                    let (layout, scene) = crate::viewport::layout_page(
                        &request.page,
                        &request.styles,
                        request.viewport,
                        fonts,
                        &mut rasterizer,
                    );
                    passes += 1;
                    let ready = Ready {
                        navigation: request.navigation,
                        revision: request.revision,
                        viewport: request.viewport,
                        layout,
                        scene,
                        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                        passes,
                    };
                    if done.send(ready).is_err() {
                        break;
                    }
                    wake();
                }
            })?;
        Ok(Self {
            commands: Some(tx),
            results,
            worker: Some(worker),
        })
    }
    pub fn request(&self, request: Request) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(request);
        }
    }
    pub fn drain(&self) -> Vec<Ready> {
        self.results.try_iter().collect()
    }
    pub fn shutdown(&mut self) {
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for LayoutWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}
