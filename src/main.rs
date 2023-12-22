use std::{
	sync::mpsc::{channel, Receiver, Sender, TryRecvError},
	thread::sleep,
	time::Duration,
};

use capture::{CameraEvent, CameraThread, Effect};
use eframe::{
	egui::{self, CentralPanel, Margin, TextureOptions, TopBottomPanel, ViewportBuilder},
	epaint::{Color32, ColorImage, Rounding, Shadow, Stroke, TextureHandle, Vec2},
};
use nokhwa::{nokhwa_check, nokhwa_initialize, utils::ApiBackend};

mod capture;
mod nv12scary;
mod vex;

fn main() -> Result<(), eframe::Error> {
	nokhwa_initialize(|_b| {});

	/*for _ in 0..1 {
		match nokhwa::query(ApiBackend::AVFoundation) {
			Err(e) => {
				println!("query failed! {e}");
			}
			Ok(query) => {
				println!("Camera Query:");
				for q in query {
					println!("\t{} [{}] {}", q.index(), q.misc(), q.human_name());
				}
			}
		};

		//println!("sleeping for a second!");
		sleep(Duration::from_secs(1));
	}*/

	let options = eframe::NativeOptions {
		viewport: ViewportBuilder::default().with_inner_size((640.0, 434.0)),
		..Default::default()
	};

	eframe::run_native("trichloride", options, Box::new(|_cc| Box::new(App::new())))
}

enum Cl3Events {
	FrameReceive,
}

struct App {
	rx: Receiver<Cl3Events>,
	preview: Option<TextureHandle>,
	effect: Effect,

	camera_thread: CameraThread,
	camera_sender: Sender<CameraEvent>,
}

impl App {
	fn new() -> Self {
		let (tx, rx) = channel();
		let camera = CameraThread::new(tx);

		Self {
			rx,
			preview: None,
			effect: Effect::Normal,

			camera_sender: camera.camera_tx(),
			camera_thread: camera,
		}
	}

	fn display_preview(&mut self, ui: &mut egui::Ui) {
		let tex = self.preview.get_or_insert_with(|| {
			ui.ctx().load_texture(
				"preview",
				ColorImage::new([1280, 720], Color32::BLACK),
				TextureOptions::default(),
			)
		});

		let avsize = ui.available_size();
		// we only want to take up 75% of the available width
		//avsize.y *= 0.75;
		let aspect = tex.aspect_ratio();

		// "Width Major" - when the width is larger (aspect ratio > 1)
		let wm_x = avsize.x;
		let wm_y = wm_x * (1.0 / aspect);

		// "Height Major" - when the height is larger (aspect ratio < 1)
		let hm_y = avsize.y;
		let hm_x = hm_y * aspect;

		let tsize = match (aspect > 1.0, wm_y > avsize.y, hm_x > avsize.x) {
			(true, false, _) | (false, _, true) => Vec2::new(wm_x, wm_y),
			(true, true, _) | (false, _, false) => Vec2::new(hm_x, hm_y),
		};

		ui.allocate_ui_with_layout(
			Vec2::new(avsize.x, tsize.y),
			egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
			|ui| ui.image((tex.id(), tsize)),
		);
	}

	fn start_preview(&mut self, ctx: &egui::Context) {
		self.camera_thread.start(ctx.clone());
	}

	fn stop_preview(&mut self) {
		self.camera_thread.stop();
	}

	fn start_recording(&mut self, ctx: &egui::Context) {
		self.camera_thread.start_recording(ctx.clone());
	}

	fn stop_recording(&mut self) {
		self.camera_thread.stop_recording();
	}
}

impl eframe::App for App {
	fn update(&mut self, ctx: &eframe::egui::Context, eframe: &mut eframe::Frame) {
		match self.rx.try_recv() {
			Ok(Cl3Events::FrameReceive) => {
				if let Some(preview) = self.preview.as_mut() {
					let lock = self.camera_thread.frame();
					let clrimg = ColorImage::from_rgb([lock.width, lock.height], &lock.data);
					preview.set(clrimg, TextureOptions::default());
				}
			}
			Err(TryRecvError::Disconnected) => unreachable!(),
			Err(TryRecvError::Empty) => {}
		}

		let central = egui::Frame {
			inner_margin: Margin::ZERO,
			outer_margin: Margin::ZERO,
			rounding: Rounding::ZERO,
			shadow: Shadow::NONE,
			fill: Color32::from_rgb(20, 20, 20),
			stroke: Stroke::NONE,
		};

		let innerframe = egui::Frame {
			inner_margin: Margin::same(4.0),
			outer_margin: Margin::ZERO,
			rounding: Rounding::ZERO,
			shadow: Shadow::NONE,
			fill: Color32::from_rgb(20, 20, 20),
			stroke: Stroke::NONE,
		};

		TopBottomPanel::bottom("bottom_panel")
			.resizable(true)
			.frame(innerframe)
			.show(ctx, |ui| {
				egui::ScrollArea::vertical()
					.auto_shrink(false)
					.show(ui, |ui| {
						ui.horizontal(|ui| {
							if self.camera_thread.running() {
								let button = egui::Button::new("Stop preview");

								if self.camera_thread.recording() {
									ui.add_enabled(false, button);
								} else if ui.add(button).clicked() {
									self.stop_preview();
								}
							} else if ui.button("Start Preview").clicked() {
								self.start_preview(ctx);
							}

							if self.camera_thread.recording() {
								if ui.button("Stop recording").clicked() {
									self.stop_recording();
								}
							} else if ui.button("Start recording").clicked() {
								self.start_recording(ctx);
							}

							let mut selected_effect = self.effect;
							ui.label("Effect");
							egui::ComboBox::from_id_source("effect_selection")
								.width(125.0)
								.selected_text(selected_effect.to_string())
								.show_ui(ui, |ui| {
									ui.selectable_value(
										&mut selected_effect,
										Effect::Normal,
										Effect::Normal.to_string(),
									);
									ui.selectable_value(
										&mut selected_effect,
										Effect::TricrideoGrey,
										Effect::TricrideoGrey.to_string(),
									);
									ui.selectable_value(
										&mut selected_effect,
										Effect::TricrideoColour,
										Effect::TricrideoColour.to_string(),
									)
								});

							if selected_effect != self.effect {
								self.effect = selected_effect;
								self.camera_sender
									.send(CameraEvent::ChangeEffect(selected_effect));
							}
						});
					});
			});

		CentralPanel::default().frame(central).show(ctx, |ui| {
			self.display_preview(ui);
		});
	}

	fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
		if self.camera_thread.running() {
			self.camera_thread.stop();
		}
	}
}
