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

use fluffy::{
	event::Event,
	event_loop::{ControlFlow, EventLoopProxy},
	Buffer, FluffyWindow, PhysicalSize, WindowBuilder,
};
use mp4::{AvcConfig, Bytes, MediaConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use nokhwa::{
	pixel_format::RgbFormat,
	utils::{CameraIndex, RequestedFormat, RequestedFormatType},
	Camera,
};
use openh264::{
	encoder::{Encoder, EncoderConfig},
	formats::YUVBuffer,
};

fn main() {
	let wbuilder = WindowBuilder::new()
		.with_title("trichrideo")
		.with_inner_size(PhysicalSize::new(640, 360));

	let mut fluff = FluffyWindow::build_window(wbuilder);

	let shutdown = Arc::new(AtomicBool::new(false));

	let el = fluff.take_el();
	let proxy = el.create_proxy();

	println!("Getting camera!");
	let mut camera = start_camera(proxy, shutdown.clone());

	println!("Starting h264 output thread");
	let (tx, rx) = channel();
	let mut h264 = start_mp4_h264_writer(camera.shared_frame.clone(), shutdown.clone(), rx);

	el.run(move |event, _, flow| {
		*flow = ControlFlow::Wait;

		match event {
			Event::RedrawRequested(_) => {
				fluff.draw_buffer();
			}
			Event::LoopDestroyed => {
				shutdown.store(true, Ordering::Release);
				camera.join();
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
							let new = rgb[idx * 3];
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
	// https://github.com/alfg/mp4-rust/blob/master/examples/mp4writer.rs
	let config = Mp4Config {
		major_brand: "isom".parse().unwrap(),
		minor_version: 512,
		compatible_brands: vec![
			str::parse("isom").unwrap(),
			str::parse("iso2").unwrap(),
			str::parse("avc1").unwrap(),
			str::parse("mp41").unwrap(),
		],
		timescale: 1000,
	};

	let file = File::create("out.mp4").unwrap();
	let mut writer = Mp4Writer::write_start(file, &config).unwrap();

	let track_config = TrackConfig {
		track_type: mp4::TrackType::Video,
		timescale: 1000,
		language: String::from("en"),
		media_conf: MediaConfig::AvcConfig(AvcConfig {
			width: 1280,
			height: 720,
			seq_param_set: vec![],
			pic_param_set: vec![],
		}),
	};

	writer.add_track(&track_config).unwrap();

	let mut encoder = None;
	let mut ticks = 0;
	let tps = 1000 / 30;

	loop {
		if shutdown.load(Ordering::Relaxed) {
			break;
		}

		match rx.recv() {
			Err(_e) => (),
			Ok(_) => {
				let read = frame.read().unwrap();

				if let None = encoder {
					encoder = Some((
						Encoder::with_config(EncoderConfig::new(
							read.width as u32,
							read.height as u32,
						))
						.unwrap(),
						YUVBuffer::new(read.width, read.height),
					));
				}

				let mut rgb = vec![0; read.width * read.height * 3];
				let (encoder, buffer) = encoder.as_mut().unwrap();
				read.as_rgb_bytes(&mut rgb);
				buffer.read_rgb(&rgb);
				let stream = encoder.encode(buffer).unwrap().to_vec();

				let sample = Mp4Sample {
					start_time: ticks,
					duration: tps,
					rendering_offset: 0,
					is_sync: false,
					bytes: Bytes::from(stream),
				};
				ticks += tps as u64;

				writer.write_sample(0, &sample).unwrap();
			}
		}
	}

	writer.write_end().unwrap();
}
