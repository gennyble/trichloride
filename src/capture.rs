use core::fmt;
use std::{
	borrow::BorrowMut,
	fmt::write,
	fs::File,
	ops::Deref,
	sync::{
		atomic::{AtomicBool, Ordering},
		mpsc::{channel, Receiver, Sender, TryRecvError},
		Arc, RwLock, RwLockReadGuard,
	},
	thread::{self, JoinHandle},
};

use devout::{Devout, Framerate};
use eframe::{egui, epaint::mutex::Mutex};
use nokhwa::{
	pixel_format::RgbFormat,
	utils::{CameraIndex, RequestedFormat, RequestedFormatType},
	Camera,
};

use crate::{
	vex::{Tricrideo, Vex},
	Cl3Events,
};

/*
gen:
the preview/record works pretty well now!

you were working on getting a good effect ssytem built out. probably an Effect
trait with frame_in() frame_out(). perhaps that's what owns the buffers?

goodnight, i love you <3
*/

/// An RGB frame of video
pub struct Frame {
	pub data: Vec<u8>,
	pub width: usize,
	pub height: usize,
}

impl Frame {
	pub fn borrow(&self) -> BorrowedFrame {
		BorrowedFrame {
			data: &self.data,
			width: self.width,
			height: self.height,
		}
	}
}

/// An RGB frame of video
pub struct BorrowedFrame<'a> {
	pub data: &'a [u8],
	pub width: usize,
	pub height: usize,
}

impl<'a> BorrowedFrame<'a> {
	pub fn to_owned(self) -> Frame {
		Frame {
			data: self.data.to_owned(),
			width: self.width,
			height: self.height,
		}
	}
}

#[derive(Copy, Clone, Debug, Hash, PartialEq)]
pub enum Effect {
	Normal,
	TricrideoGrey,
	TricrideoColour,
}

impl fmt::Display for Effect {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Effect::Normal => write!(f, "Normal"),
			Effect::TricrideoGrey => write!(f, "Grey Trichrome"),
			Effect::TricrideoColour => write!(f, "Colour Trichrome"),
		}
	}
}

pub enum CameraEvent {
	ChangeEffect(Effect),
	RecordingStarted,
	RecordingStopped,
	Shutdown,
}

/// Owner the webcam capture and video encoding threads and everything
/// to communicate between them and the GUI thread.
pub struct CameraThread {
	gui_tx: Sender<Cl3Events>,

	shared_frame: Arc<RwLock<Frame>>,
	effect: Arc<Mutex<Effect>>,
	camera: RespawnableThread<CameraEvent>,
	encoder: RespawnableThread<MuxerEvent>,
}

impl CameraThread {
	pub fn new(sender: Sender<Cl3Events>) -> Self {
		Self {
			gui_tx: sender,

			shared_frame: Arc::new(RwLock::new(Frame {
				data: vec![],
				width: 0,
				height: 0,
			})),
			effect: Arc::new(Mutex::new(Effect::Normal)),
			camera: RespawnableThread::new(),
			encoder: RespawnableThread::new(),
		}
	}

	pub fn camera_tx(&self) -> Sender<CameraEvent> {
		self.camera.tx.clone()
	}

	/// Starts capturing frames from the camera. The [egui::Context] `ctx` is
	/// used to wakeup the GUI so it can receive the new frame.
	pub fn start(&mut self, ctx: egui::Context) {
		if self.camera.running() {
			return;
		}

		let frame = self.shared_frame.clone();
		let effect = self.effect.clone();
		let gui_tx = self.gui_tx.clone();
		let encoder_tx = self.encoder.tx.clone();
		self.camera.start(|rx| {
			thread::spawn(move || camera_runner(ctx, frame, effect, rx, gui_tx, encoder_tx))
		});
	}

	/// Shuts down, if alive, the camera thread and then the recording thread.
	pub fn stop(&mut self) {
		self.camera.tx.send(CameraEvent::Shutdown);
		self.camera.join();
		self.stop_recording();
	}

	pub fn start_recording(&mut self, ctx: egui::Context) {
		if self.encoder.running() {
			return;
		}
		self.start(ctx);

		let frame = self.shared_frame.clone();
		self.encoder
			.start(|rx| thread::spawn(|| mp4_h264_writer(frame, rx)));

		self.camera.tx.send(CameraEvent::RecordingStarted);
	}

	pub fn stop_recording(&mut self) {
		if self.encoder.running() {
			self.camera.tx.send(CameraEvent::RecordingStopped);
			self.encoder.tx.send(MuxerEvent::Shutdown).unwrap();
			self.encoder.join();
		}
	}

	pub fn running(&self) -> bool {
		self.camera.running()
	}

	pub fn recording(&self) -> bool {
		self.encoder.running()
	}

	pub fn frame(&self) -> RwLockReadGuard<Frame> {
		self.shared_frame.read().unwrap()
	}
}

pub const FRAMERATE: u32 = 30;

fn camera_runner(
	ctx: egui::Context,
	frame: Arc<RwLock<Frame>>,
	effect_type: Arc<Mutex<Effect>>,
	camera_rx: Receiver<CameraEvent>,
	gui_tx: Sender<Cl3Events>,
	encoder_tx: Sender<MuxerEvent>,
) -> Receiver<CameraEvent> {
	let requested_format =
		RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution);
	let mut camera = Camera::new(CameraIndex::Index(0), requested_format).unwrap();

	println!(
		"Got camera: {} {}",
		camera.index(),
		camera.info().human_name()
	);

	let width = camera.camera_format().width();
	let height = camera.camera_format().height();
	// working buffer
	let mut rgb = vec![0; width as usize * height as usize * 3];

	// Can't move Camera between threads, so we set details here
	{
		let mut lock = frame.write().unwrap();

		lock.width = width as usize;
		lock.height = height as usize;
		lock.data.resize(width as usize * height as usize * 3, 0);
	}

	println!("Opening stream...");
	camera.open_stream().unwrap();
	println!("Opened! Entering loop");

	// This is never used outside of this function. Keeping the lock here is safe
	let mut effect_type = effect_type.lock();
	let mut effect: Option<Box<dyn Vex>> = None;
	let mut recording = false;
	let mut shutdown = false;

	loop {
		let mut effect_changed = false;
		let camera_frame = camera.frame_raw();

		// Make sure we don't leave events in the receiver
		loop {
			match camera_rx.try_recv() {
				Err(TryRecvError::Empty) => break,
				Err(TryRecvError::Disconnected) => panic!("camera sender disconnected??"),
				Ok(CameraEvent::Shutdown) => shutdown = true,
				Ok(CameraEvent::RecordingStarted) => recording = true,
				Ok(CameraEvent::RecordingStopped) => recording = false,
				Ok(CameraEvent::ChangeEffect(effect)) => {
					effect_changed = true;
					*effect_type = effect;
				}
			}
		}

		if effect_changed {
			match *effect_type {
				Effect::Normal => effect = None,
				Effect::TricrideoGrey => {
					let frame = effect.as_mut().map(|v| v.frame_out().to_owned());
					if let Some(frame) = frame {
						effect = Some(Box::new(Tricrideo::from_frame(frame)));
					} else {
						effect = Some(Box::new(Tricrideo::new(width as usize, height as usize)));
					}
				}
				Effect::TricrideoColour => {
					let frame = effect.as_mut().map(|v| v.frame_out().to_owned());
					let mut tri = if let Some(frame) = frame {
						Tricrideo::from_frame(frame)
					} else {
						Tricrideo::new(width as usize, height as usize)
					};
					tri.set_coloured(true);
					effect = Some(Box::new(tri));
				}
			}
		}

		match camera_frame {
			Err(_e) => (),
			Ok(cow) => {
				crate::nv12scary::yuv422_rgb(&cow, &mut rgb, width as usize);
				//let buffer = cow.decode_image::<RgbFormat>().unwrap();
				//println!("width = {} | height = {}", buffer.width(), buffer.height());
				//let buff = buffer.into_raw();
				//rgb.copy_from_slice(&buff);

				{
					let mut lock = frame.write().unwrap();

					let data = if let Some(effect) = effect.as_mut() {
						let brwd = BorrowedFrame {
							data: &rgb,
							width: width as usize,
							height: height as usize,
						};

						effect.effect(brwd).data
					} else {
						&rgb
					};

					unsafe {
						std::ptr::copy_nonoverlapping(
							data.as_ptr(),
							lock.data.as_mut_ptr(),
							data.len(),
						)
					}
				}

				if recording {
					encoder_tx.send(MuxerEvent::FrameReceive).unwrap();
				}

				ctx.request_repaint();
				gui_tx.send(Cl3Events::FrameReceive).unwrap();
			}
		}

		if shutdown {
			break camera_rx;
		}
	}
}

enum MuxerEvent {
	FrameReceive,
	Shutdown,
}

fn mp4_h264_writer(frame: Arc<RwLock<Frame>>, rx: Receiver<MuxerEvent>) -> Receiver<MuxerEvent> {
	let file = File::create("out.mp4").unwrap();
	let mut h264 = Devout::new(file, Framerate::Whole(FRAMERATE));

	loop {
		match rx.recv() {
			Err(_e) => (),
			Ok(MuxerEvent::FrameReceive) => {
				let read = frame.read().unwrap();
				h264.frame(read.width as u32, read.height as u32, &read.data);
			}
			Ok(MuxerEvent::Shutdown) => {
				h264.done();
				break rx;
			}
		}
	}
}

struct RespawnableThread<E> {
	tx: Sender<E>,
	rx: Option<Receiver<E>>,
	handle: Option<JoinHandle<Receiver<E>>>,
}

impl<E> RespawnableThread<E> {
	fn new() -> Self {
		let (tx, rx) = channel();

		Self {
			tx,
			rx: Some(rx),
			handle: None,
		}
	}

	fn join(&mut self) {
		if let Some(handle) = self.handle.take() {
			let rx = handle.join().unwrap();
			self.rx = Some(rx);
		}
	}

	fn running(&self) -> bool {
		self.handle.is_some()
	}

	fn start<F>(&mut self, starter: F)
	where
		F: FnOnce(Receiver<E>) -> JoinHandle<Receiver<E>>,
	{
		let rx = self.rx.take().unwrap();
		let handle = starter(rx);
		self.handle = Some(handle);
	}
}
