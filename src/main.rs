mod nv12scary;

use std::{
	fs::File,
	sync::{
		atomic::{AtomicBool, Ordering},
		mpsc::{channel, Receiver},
		Arc, RwLock,
	},
	thread::JoinHandle,
};

use devout::{Devout, Framerate};
use fluffy::{
	event::Event,
	event_loop::{ControlFlow, EventLoopProxy},
	Buffer, FluffyWindow, PhysicalSize, WindowBuilder,
};
use nokhwa::{
	pixel_format::RgbFormat,
	utils::{CameraIndex, RequestedFormat, RequestedFormatType},
	Camera,
};

fn main() {
	let wbuilder = WindowBuilder::new()
		.with_title("trichlroide")
		.with_inner_size(PhysicalSize::new(640, 360));

	let mut fluff = FluffyWindow::build_window(wbuilder);

	let shutdown = Arc::new(AtomicBool::new(false));

	let el = fluff.take_el();
	let proxy = el.create_proxy();

	println!("Getting camera!");
	let mut camera = start_camera(proxy, shutdown.clone());

	println!("Starting h264 output thread");
	let (tx, rx) = channel();
	let mut h264 = Some(start_mp4_h264_writer(
		camera.shared_frame.clone(),
		shutdown.clone(),
		rx,
	));

	el.run(move |event, _, flow| {
		*flow = ControlFlow::Wait;

		match event {
			Event::RedrawRequested(_) => {
				fluff.draw_buffer();
			}
			Event::LoopDestroyed => {
				println!("Shutting down!");
				shutdown.store(true, Ordering::Release);

				// We need to unblock the h264 thread by sending once more
				tx.send(()).unwrap();

				println!("Stored shutdown");
				camera.join();
				println!("Joined camera");
				h264.take().unwrap().join().unwrap();
				println!("Done!");
			}
			Event::UserEvent(()) => {
				tx.send(()).unwrap();

				// Frame received! Shove it in our buffer and request redraw
				let read = camera.shared_frame.read().unwrap();
				let scaled = neam::nearest(
					read.data.as_slice(),
					1,
					read.width as u32,
					read.height as u32,
					fluff.buffer.width as u32,
					fluff.buffer.height as u32,
				);

				fluff.buffer.data = scaled;
				fluff.window.request_redraw();
			}
			_ => (),
		}

		fluff.common_events(&event, flow);
	});
}

fn get_camera() -> Camera {
	let requested_format =
		RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution);

	let camera = Camera::new(CameraIndex::Index(0), requested_format).unwrap();

	camera
}

pub struct CameraThread {
	pub shared_frame: Arc<RwLock<Buffer>>,
	pub handle: Option<JoinHandle<()>>,
}

impl CameraThread {
	// If the camera thread is alive, it joins. Otherwise it does nothing
	pub fn join(&mut self) {
		if let Some(handle) = self.handle.take() {
			handle.join().unwrap();
		}
	}
}

pub fn start_camera(proxy: EventLoopProxy<()>, shutdown: Arc<AtomicBool>) -> CameraThread {
	let frame = Buffer::new(1, 1);
	let shared_frame = Arc::new(RwLock::new(frame));

	let shared = shared_frame.clone();
	let handle = std::thread::spawn(move || camera_runner(proxy, shutdown, shared));
	println!("Camera thread spanwed!");

	CameraThread {
		shared_frame,
		handle: Some(handle),
	}
}

pub const COLOUR: bool = true;

pub fn camera_runner(
	proxy: EventLoopProxy<()>,
	shutdown: Arc<AtomicBool>,
	frame: Arc<RwLock<Buffer>>,
) {
	let mut camera = get_camera();
	println!("Got camera!");

	let width = camera.camera_format().width();
	let height = camera.camera_format().height();
	let mut rgb = vec![0; width as usize * height as usize * 3];

	// Can't move Camera between threads, so we set the res here
	{
		frame
			.write()
			.unwrap()
			.resize(width as usize, height as usize);
	}

	// 0 Red, 1 Green, 2 Blue.
	let mut rgb_idx = 0;

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
				nv12scary::yuv422_rgb(&cow, &mut rgb, width as usize);

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

				rgb_idx = (rgb_idx + 1) % 3;

				proxy.send_event(()).unwrap();
			}
		}
	}
}

pub fn start_mp4_h264_writer(
	frame: Arc<RwLock<Buffer>>,
	shutdown: Arc<AtomicBool>,
	rx: Receiver<()>,
) -> JoinHandle<()> {
	std::thread::spawn(move || mp4_h264_writer(frame, shutdown, rx))
}

pub fn mp4_h264_writer(frame: Arc<RwLock<Buffer>>, shutdown: Arc<AtomicBool>, rx: Receiver<()>) {
	let file = File::create("out.mp4").unwrap();
	let mut h264 = Devout::new(file, Framerate::Thirty);
	let mut rgb: Vec<u8> = vec![];

	loop {
		if shutdown.load(Ordering::Relaxed) {
			println!("Doing");
			h264.done();
			println!("Did!");
			return;
		}

		match rx.recv() {
			Err(_e) => (),
			Ok(_) => {
				println!("before read acq");
				let read = frame.read().unwrap();

				let frame_size = read.width * read.height * 3;
				if rgb.len() < frame_size {
					rgb.resize(frame_size, 0);
				}

				read.as_rgb_bytes(&mut rgb);

				h264.frame(read.width as u32, read.height as u32, &rgb);
				println!("after frame");
			}
		}
	}
}
