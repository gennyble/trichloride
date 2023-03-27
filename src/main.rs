mod nv12scary;

use std::{
	fs::File,
	io::Write,
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
	let mut h264 = Outh264::new();

	loop {
		if shutdown.load(Ordering::Relaxed) {
			println!("Doing");
			h264.done();
			println!("Did!");
			break;
		}

		match rx.recv() {
			Err(_e) => (),
			Ok(_) => {
				println!("before read acq");
				let read = frame.read().unwrap();
				h264.frame(&read);
				println!("after frame");
			}
		}
	}
}

struct Maybeh264 {
	encoder: Encoder,
	rgb: Vec<u8>,
	yuvbuffer: YUVBuffer,
}

pub struct Outh264 {
	maybe: Option<Maybeh264>,
	mp4: Option<Mp4Writer<File>>,
	ticks: u64,
	idk: u8,
	nals: u8,
	bfr: Vec<u8>,
}

impl Outh264 {
	const TIMESCALE: u64 = 15360;
	/// 30fps
	const TPS: u32 = Self::TIMESCALE as u32 / 30;

	pub fn new() -> Self {
		Self {
			maybe: None,
			mp4: None,
			ticks: 0,
			idk: 0,
			nals: 0,
			bfr: vec![],
		}
	}

	pub fn frame(&mut self, buffer: &Buffer) {
		let yes = match self.maybe.as_mut() {
			None => {
				let encoder = Encoder::with_config(EncoderConfig::new(
					buffer.width as u32,
					buffer.height as u32,
				))
				.unwrap();
				let yuvbuffer = YUVBuffer::new(buffer.width, buffer.height);
				let rgb = vec![0; buffer.width * buffer.height * 3];

				self.maybe = Some(Maybeh264 {
					encoder,
					rgb,
					yuvbuffer,
				});

				self.maybe.as_mut().unwrap()
			}
			Some(yes) => yes,
		};

		buffer.as_rgb_bytes(&mut yes.rgb);
		yes.yuvbuffer.read_rgb(&yes.rgb);
		let out = yes.encoder.encode(&yes.yuvbuffer).unwrap().to_vec();

		let writer = match self.mp4.as_mut() {
			None => {
				// We need to make it, oh no
				// Find the SPS and Pps
				let nals = Self::nal_split(&out);
				let sps = nals.iter().find(|nal| nal.kind == NalType::Sps).unwrap(); //.as_full_unit();

				//println!("SPS: {sps:?}");

				let pps = nals.iter().find(|nal| nal.kind == NalType::Pps).unwrap(); //.as_full_unit();

				//println!("PPS: {sps:?}");

				let writer = Self::make_mp4(&sps.as_bare(), &pps.as_bare());
				self.mp4 = Some(writer);
				self.mp4.as_mut().unwrap()
			}
			Some(mp4) => mp4,
		};

		let nals = Self::nal_split(&out);

		println!("{:.03}s", self.ticks as f32 / Self::TIMESCALE as f32);

		if nals.len() == 0 {
			println!("No NALS. Out len {}", out.len());
			self.idk += 1;
			return;
		}

		let mut data = vec![];
		for n in nals {
			println!("NAL Type - {:?}", n.kind);
			match n.kind {
				NalType::Other(_) => {
					//data.extend_from_slice(&n.as_length_prefixed());
				}
				_ => (),
			}
			data.extend_from_slice(&n.as_length_prefixed());
			//self.nals += 1;
		}

		//if self.nals > 5 {
		let sample = Mp4Sample {
			start_time: self.ticks,
			duration: Self::TPS,
			rendering_offset: 0,
			is_sync: false,
			bytes: Bytes::from(data), //bytes: Bytes::from(self.bfr.drain(..).collect::<Vec<u8>>()),
		};
		self.nals = 0;
		self.ticks += Self::TPS as u64;
		println!("Wrote sample!");

		writer.write_sample(1, &sample).unwrap();
		//}
	}

	pub fn done(&mut self) {
		if let Some(mut mp4) = self.mp4.take() {
			mp4.write_end().unwrap();
			mp4.into_writer().flush().unwrap();
		}
	}

	// We need to find the Sequence Parameter Set and Picture Parameter Set
	// https://amble.quest/notice/AU1JHnpqbb7L2LgiaO

	fn nal_split<'a>(buf: &'a [u8]) -> Vec<Nal<'a>> {
		let mut nal = &buf[..];

		let mut ret = vec![];
		loop {
			//println!("{}", nal.len());
			if nal.len() == 0 {
				break;
			}

			let lead_type = match &nal[0..nal.len().min(4)] {
				[0, 0, 0, 1] => {
					nal = &nal[4..];
					LeadType::Null3
				}
				[0, 0, 1, _] => {
					nal = &nal[3..];
					LeadType::Null2
				}
				_ => {
					println!("NO NAL AHHHHHHH");
					break;
				}
			};

			let nal_id = nal[0];
			// "NAL AND ID" (the ID bitwise anded with 0x1F)
			let nal_aid = nal_id & 0x1F;
			//println!("{nal_id} or with the bitwise and, {}", nal_id & 0x1F);

			let kind = match nal_aid {
				7 => NalType::Sps,
				8 => NalType::Pps,
				k => NalType::Other(k),
			};

			nal = &nal[1..];

			// Find the next header. I should do this in the "main loop" but I already didn't and this is easier now
			let mut len = 1;
			'find_next: loop {
				match &nal[len..] {
					[0, 0, 1, ..] | [0, 0, 0, 1, ..] => {
						ret.push(Nal {
							lead_type,
							kind,
							raw_nal_kind: nal_id,
							data: &nal[..len],
						});
						nal = &nal[len..];

						break 'find_next;
					}
					[] => {
						ret.push(Nal {
							lead_type,
							kind,
							raw_nal_kind: nal_id,
							data: &nal[..],
						});
						nal = &[];

						break 'find_next;
					}
					_ => len += 1,
				}
			}
		}

		ret
	}

	fn make_mp4(sps: &[u8], pps: &[u8]) -> Mp4Writer<File> {
		let config = Mp4Config {
			major_brand: "isom".parse().unwrap(),
			minor_version: 512,
			compatible_brands: vec![
				str::parse("isom").unwrap(),
				str::parse("iso2").unwrap(),
				str::parse("avc1").unwrap(),
				str::parse("mp41").unwrap(),
			],
			timescale: Self::TIMESCALE as u32,
		};

		let file = File::create("out.mp4").unwrap();
		let mut writer = Mp4Writer::write_start(file, &config).unwrap();

		let track_config = TrackConfig {
			track_type: mp4::TrackType::Video,
			timescale: Self::TIMESCALE as u32,
			language: String::from("und"),
			media_conf: MediaConfig::AvcConfig(AvcConfig {
				width: 1280,
				height: 720,
				seq_param_set: sps.to_vec(),
				pic_param_set: pps.to_vec(),
			}),
		};

		println!("{sps:?}\n{pps:?}");

		writer.add_track(&track_config).unwrap();

		writer
	}
}

struct Nal<'a> {
	lead_type: LeadType,
	kind: NalType,
	raw_nal_kind: u8,
	data: &'a [u8],
}

impl<'a> Nal<'a> {
	pub fn as_full_unit(&self) -> Vec<u8> {
		let mut ret = vec![];

		match self.lead_type {
			LeadType::Null2 => ret.extend_from_slice(&[0x00, 0x00, 0x01]),
			LeadType::Null3 => ret.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]),
		}

		ret.push(self.raw_nal_kind);
		ret.extend_from_slice(self.data);
		ret
	}

	pub fn as_length_prefixed(&self) -> Vec<u8> {
		let mut ret = vec![];
		ret.extend_from_slice(&(self.data.len() as u32 + 1).to_be_bytes());
		ret.push(self.raw_nal_kind);
		ret.extend_from_slice(self.data);
		ret
	}

	pub fn as_bare(&self) -> Vec<u8> {
		let mut ret = vec![];
		ret.push(self.raw_nal_kind);
		ret.extend_from_slice(self.data);
		ret
	}
}

enum LeadType {
	Null2,
	Null3,
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
enum NalType {
	Other(u8),
	Sps,
	Pps,
}
