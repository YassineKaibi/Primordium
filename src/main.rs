//! Entry point: load config, spawn the sim thread, run the window loop.
//!
//! The simulation runs on its own thread as fast as it can and publishes a
//! `WorldSnapshot` through an `ArcSwap`. The main thread owns the window and
//! the renderer, and draws whatever the latest published snapshot is. Neither
//! side blocks the other: a slow renderer drops frames, a slow sim just gets
//! drawn twice.

// The `@veridikt` convention (see CLAUDE.md) puts an annotation block after a
// `///` doc comment, separated by a blank line, directly above the item. That
// is exactly the shape this lint fires on, 54 times across the tree, so the
// lint and the project's documented convention cannot both hold. The
// convention wins; if the annotations ever move above the doc comments, drop
// this.
#![allow(clippy::empty_line_after_doc_comments)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use pixels::{Pixels, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use primordium::config::WorldConfig;
use primordium::render::Renderer;
use primordium::render::color::ColorMode;
use primordium::sim::Simulation;
use primordium::sim::world::WorldSnapshot;

/// Shared state between the sim thread and the window.

// @veridikt
// kind: type
// name: SimHandle
// purpose: "The only channel between sim thread and window: the latest snapshot plus the controls the window can set"
// because: "arc-swap lets the renderer take the newest snapshot without ever blocking the sim thread, which is what keeps the window smooth while ticks run flat out"
struct SimHandle {
    latest: ArcSwap<WorldSnapshot>,
    /// Ticks to run per published snapshot. Fast-forward without redrawing.
    speed: AtomicU32,
    paused: AtomicBool,
    running: AtomicBool,
    /// Bumped by the window to restart the run from a fresh seed.
    reseed: AtomicU64,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let config = load_config(&args)?;

    println!(
        "Primordium — {}x{} grid, {} cells, seed {}",
        config.grid_width, config.grid_height, config.initial_cell_count, config.seed
    );
    println!(
        "  space pause · 1 genetic · 2 strategy · 3 phase · 4 energy · \
         ↑/↓ speed · r reseed · esc quit"
    );

    let mut simulation = Simulation::new(config.clone());
    let handle = Arc::new(SimHandle {
        latest: ArcSwap::from_pointee(simulation.snapshot()),
        speed: AtomicU32::new(1),
        paused: AtomicBool::new(false),
        running: AtomicBool::new(true),
        reseed: AtomicU64::new(0),
    });

    let sim_handle = Arc::clone(&handle);
    let sim_config = config.clone();
    let sim_thread = std::thread::Builder::new()
        .name("primordium-sim".into())
        .spawn(move || {
            let mut generation = 0u64;
            while sim_handle.running.load(Ordering::Relaxed) {
                let requested = sim_handle.reseed.load(Ordering::Relaxed);
                if requested != generation {
                    generation = requested;
                    let mut restarted = sim_config.clone();
                    restarted.seed = sim_config.seed.wrapping_add(generation);
                    simulation = Simulation::new(restarted);
                    sim_handle.latest.store(Arc::new(simulation.snapshot()));
                }
                if sim_handle.paused.load(Ordering::Relaxed) {
                    // Nothing to compute; don't spin a core for it.
                    std::thread::sleep(std::time::Duration::from_millis(16));
                    continue;
                }
                let steps = sim_handle.speed.load(Ordering::Relaxed).max(1);
                for _ in 0..steps {
                    simulation.step();
                }
                sim_handle.latest.store(Arc::new(simulation.snapshot()));
            }
        })
        .context("failed to spawn sim thread")?;

    let event_loop = EventLoop::new().context("failed to create event loop")?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App {
        config,
        handle: Arc::clone(&handle),
        window: None,
        pixels: None,
        renderer: Renderer::new(0, 0),
    };
    let result = event_loop.run_app(&mut app).context("event loop failed");

    handle.running.store(false, Ordering::Relaxed);
    handle.paused.store(false, Ordering::Relaxed);
    let _ = sim_thread.join();
    result
}

/// Load `WorldConfig` from a JSON path given on the command line, or fall
/// back to the defaults.
fn load_config(args: &[String]) -> Result<WorldConfig> {
    let Some(path) = args.get(1) else {
        return Ok(WorldConfig::default());
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading config file {path}"))?;
    serde_json::from_str(&text).with_context(|| format!("parsing config file {path}"))
}

struct App {
    config: WorldConfig,
    handle: Arc<SimHandle>,
    window: Option<Arc<Window>>,
    pixels: Option<Pixels>,
    renderer: Renderer,
}

impl App {
    /// Report the current view in the title bar — the sim has no HUD, and
    /// this is where "is anything alive?" gets answered.
    fn update_title(&self, snapshot: &WorldSnapshot) {
        let Some(window) = &self.window else { return };
        let mode = match self.renderer.mode {
            ColorMode::Genetic => "genetic",
            ColorMode::Strategy => "strategy",
            ColorMode::Phase => "phase",
            ColorMode::Energy => "energy",
        };
        let speed = self.handle.speed.load(Ordering::Relaxed);
        let paused = if self.handle.paused.load(Ordering::Relaxed) {
            " [paused]"
        } else {
            ""
        };
        window.set_title(&format!(
            "Primordium — tick {} · pop {} · {}x · {}{}",
            snapshot.tick, snapshot.stats.population, speed, mode, paused
        ));
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (w, h) = (self.config.grid_width, self.config.grid_height);
        let attrs = Window::default_attributes()
            .with_title("Primordium")
            .with_inner_size(winit::dpi::LogicalSize::new(w as f64, h as f64));
        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                eprintln!("could not create window: {err}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        let surface = SurfaceTexture::new(size.width.max(1), size.height.max(1), window.as_ref());
        match Pixels::new(w, h, surface) {
            Ok(pixels) => self.pixels = Some(pixels),
            Err(err) => {
                eprintln!("could not create pixel surface: {err}");
                event_loop.exit();
                return;
            }
        }
        self.renderer = Renderer::new(w, h);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(pixels) = &mut self.pixels
                    && let Err(err) = pixels.resize_surface(size.width.max(1), size.height.max(1))
                {
                    eprintln!("resize failed: {err}");
                    event_loop.exit();
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                match code {
                    KeyCode::Escape => event_loop.exit(),
                    KeyCode::Space => {
                        let paused = self.handle.paused.load(Ordering::Relaxed);
                        self.handle.paused.store(!paused, Ordering::Relaxed);
                    }
                    KeyCode::Digit1 => self.renderer.set_mode(ColorMode::Genetic),
                    KeyCode::Digit2 => self.renderer.set_mode(ColorMode::Strategy),
                    KeyCode::Digit3 => self.renderer.set_mode(ColorMode::Phase),
                    KeyCode::Digit4 => self.renderer.set_mode(ColorMode::Energy),
                    KeyCode::ArrowUp => {
                        let speed = self.handle.speed.load(Ordering::Relaxed);
                        self.handle
                            .speed
                            .store((speed * 2).min(1024), Ordering::Relaxed);
                    }
                    KeyCode::KeyR => {
                        self.handle.reseed.fetch_add(1, Ordering::Relaxed);
                    }
                    KeyCode::ArrowDown => {
                        let speed = self.handle.speed.load(Ordering::Relaxed);
                        self.handle
                            .speed
                            .store((speed / 2).max(1), Ordering::Relaxed);
                    }
                    _ => {}
                }
            }

            WindowEvent::RedrawRequested => {
                let snapshot = self.handle.latest.load_full();
                if let Some(pixels) = &mut self.pixels {
                    let frame = self.renderer.render(&snapshot);
                    pixels.frame_mut().copy_from_slice(&frame);
                    if let Err(err) = pixels.render() {
                        eprintln!("render failed: {err}");
                        event_loop.exit();
                        return;
                    }
                }
                self.update_title(&snapshot);
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
