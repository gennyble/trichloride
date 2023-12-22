use std::{
	fs::File,
	sync::{
		atomic::{AtomicBool, Ordering},
		mpsc::{channel, Receiver, Sender},
		Arc, RwLock, RwLockReadGuard,
	},
	thread::JoinHandle,
};

use devout::{Devout, Framerate};
use eframe::egui;
use nokhwa::{
	pixel_format::RgbFormat,
	utils::{CameraIndex, RequestedFormat, RequestedFormatType},
	Camera,
};

use crate::Cl3Events;

pub struct Frame {
	pub data: Vec<u8>,
	pub width: usize,
	pub height: usize,
}

pub struct CameraThread {
	gui_tx: Sender<Cl3Events>,
	camera_shutdown: Arc<AtomicBool>,

	shared_frame: Arc<RwLock<Frame>>,
	recording: Arc<AtomicBool>,
	camera: Option<JoinHandle<()>>,

	muxer: Option<JoinHandle<Receiver<MuxerEvents>>>,
	muxer_tx: Sender<MuxerEvents>,
	muxer_rx: Option<Receiver<MuxerEvents>>,
}

impl CameraThread {
	pub fn new(sender: Sender<Cl3Events>) -> Self {
		let (muxer_tx, muxer_rx) = channel();

		Self {
			gui_tx: sender,
			camera_shutdown: Arc::new(AtomicBool::new(false)),

			shared_frame: Arc::new(RwLock::new(Frame {
				data: vec![],
				width: 0,
				height: 0,
			})),
			recording: Arc::new(AtomicBool::new(false)),
			camera: None,

			muxer: None,
			muxer_tx,
			muxer_rx: Some(muxer_rx),
		}
	}

	/// Starts capturing frames from the camera. The [egui::Context] `ctx` is
	/// used to wakeup the GUI so it can receive the new frame.
	pub fn start(&mut self, ctx: egui::Context) {
		if self.camera.is_some() {
			return;
		}

		// make sure it doesn't immediatly shutdown
		self.camera_shutdown.store(false, Ordering::Release);

		let tx = self.gui_tx.clone();
		let shutdown = self.camera_shutdown.clone();
		let frame = self.shared_frame.clone();
		let recording = self.recording.clone();
		let mtx = self.muxer_tx.clone();
		let handle =
			std::thread::spawn(move || camera_runner(ctx, tx, shutdown, frame, recording, mtx));

		self.camera = Some(handle);
	}

	/// Shuts down, if alive, the camera thread and then the recording thread.
	pub fn stop(&mut self) {
		if let Some(handle) = self.camera.take() {
			// tell thread to shutdown
			self.camera_shutdown.store(true, Ordering::Release);
			handle.join().unwrap();
		}

		if let Some(handle) = self.muxer.take() {
			self.recording.store(false, Ordering::Release);
			self.muxer_tx.send(MuxerEvents::Shutdown).unwrap();
			let muxer_rx = handle.join().unwrap();
			self.muxer_rx = Some(muxer_rx);
		}
	}

	pub fn start_recording(&mut self, ctx: egui::Context) {
		if self.muxer.is_some() {
			return;
		}
		self.start(ctx);

		let frame = self.shared_frame.clone();
		let muxer_rx = self.muxer_rx.take().unwrap();
		let handle = std::thread::spawn(move || mp4_h264_writer(frame, muxer_rx));

		self.muxer = Some(handle);
		self.recording.store(true, Ordering::Release);
	}

	pub fn stop_recording(&mut self) {
		if let Some(handle) = self.muxer.take() {
			self.recording.store(false, Ordering::Release);
			self.muxer_tx.send(MuxerEvents::Shutdown).unwrap();
			let muxer_rx = handle.join().unwrap();
			self.muxer_rx = Some(muxer_rx);
		}
	}

	pub fn running(&self) -> bool {
		self.camera.is_some()
	}

	pub fn recording(&self) -> bool {
		self.muxer.is_some()
	}

	pub fn frame(&self) -> RwLockReadGuard<Frame> {
		self.shared_frame.read().unwrap()
	}
}

pub const FRAMERATE: u32 = 30;

fn camera_runner(
	ctx: egui::Context,
	tx: Sender<Cl3Events>,
	shutdown: Arc<AtomicBool>,
	frame: Arc<RwLock<Frame>>,
	recording: Arc<AtomicBool>,
	muxer_tx: Sender<MuxerEvents>,
) {
	let requested_format =
		RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution);
	let mut camera = Camera::new(CameraIndex::Index(0), requested_format).unwrap();

	println!("Got camera!");

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

	loop {
		if shutdown.load(Ordering::Relaxed) {
			return;
		}

		match camera.frame_raw() {
			Err(_e) => (),
			Ok(cow) => {
				crate::nv12scary::yuv422_rgb(&cow, &mut rgb, width as usize);

				{
					let mut lock = frame.write().unwrap();
					let data = &mut lock.data;

					for (idx, px) in data.chunks_mut(3).enumerate() {
						let new = ((rgb[idx * 3] as u32
							+ rgb[idx * 3 + 1] as u32 + rgb[idx * 3 + 2] as u32)
							/ 3) as u8;

						//*px = (*px >> 8) | ((new as u32) << 16);
						px[1] = px[0];
						px[2] = px[1];
						px[0] = new;
					}

					/*unsafe {
						std::ptr::copy_nonoverlapping(
							rgb.as_ptr(),
							lock.data.as_mut_ptr(),
							rgb.len(),
						)
					};*/
				}

				if recording.load(Ordering::Acquire) {
					muxer_tx.send(MuxerEvents::FrameReceive).unwrap();
				}

				ctx.request_repaint();
				tx.send(Cl3Events::FrameReceive).unwrap();
			}
		}
	}
}

enum MuxerEvents {
	FrameReceive,
	Shutdown,
}

fn mp4_h264_writer(frame: Arc<RwLock<Frame>>, rx: Receiver<MuxerEvents>) -> Receiver<MuxerEvents> {
	let file = File::create("out.mp4").unwrap();
	let mut h264 = Devout::new(file, Framerate::Whole(FRAMERATE));

	loop {
		match rx.recv() {
			Err(_e) => (),
			Ok(MuxerEvents::FrameReceive) => {
				let read = frame.read().unwrap();
				h264.frame(read.width as u32, read.height as u32, &read.data);
			}
			Ok(MuxerEvents::Shutdown) => {
				h264.done();
				break rx;
			}
		}
	}
}

/*TRICHROME EFFECT CODE:

{
	let mut buff = frame.write().unwrap();

	for (idx, px) in buff.data.iter_mut().enumerate() {
		if COLOUR {
			let channel = rgb[idx * 3 + rgb_idx];
			let mut bytes = px.to_be_bytes();
			bytes[1 + rgb_idx] = channel;
			*px = u32::from_be_bytes(bytes);
		} else {
			let new = ((rgb[idx * 3] as u32
				+ rgb[idx * 3 + 1] as u32 + rgb[idx * 3 + 2] as u32)
				/ 3) as u8;
			*px = (*px >> 8) | ((new as u32) << 16);
		}
	}
}

rgb_idx = (rgb_idx + 1) % 3;*/
