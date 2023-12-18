use std::{
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc, RwLock,
	},
	thread::JoinHandle,
	time::{Duration, Instant},
};

use nokhwa::{
	pixel_format::RgbFormat,
	utils::{CameraIndex, RequestedFormat, RequestedFormatType},
	Camera,
};
use winit::event_loop::EventLoopProxy;

pub struct Frame {
	pub data: Vec<u8>,
	pub width: usize,
	pub height: usize,
}

pub struct CameraThread {
	pub shared_frame: Arc<RwLock<Frame>>,
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

/// The camera thread will send events to `frame_notify` to tell the caller
/// that it's received a new frame
pub fn start_camera(proxy: EventLoopProxy<()>, shutdown: Arc<AtomicBool>) -> CameraThread {
	let frame = Frame {
		data: vec![],
		width: 0,
		height: 0,
	};
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
pub const FRAMERATE: u32 = 30;

pub fn camera_runner(
	proxy: EventLoopProxy<()>,
	shutdown: Arc<AtomicBool>,
	frame: Arc<RwLock<Frame>>,
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

	//TODO: use error to collect difference from last to expected MS
	let mut last = Instant::now();
	let mut error = Duration::ZERO;
	const TARGET: Duration = Duration::from_millis(1000 / FRAMERATE as u64);

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
				let now = Instant::now();
				/*let acc = error + (now - last);
				if acc < TARGET {
					continue;
				}*/

				println!("Took frame at {}ms", (now - last).as_millis());
				//let error = acc - TARGET;
				last = now;

				crate::nv12scary::yuv422_rgb(&cow, &mut rgb, width as usize);

				{
					let mut lock = frame.write().unwrap();

					unsafe {
						std::ptr::copy_nonoverlapping(
							rgb.as_ptr(),
							lock.data.as_mut_ptr(),
							rgb.len(),
						)
					};
				}

				proxy.send_event(()).unwrap();
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
