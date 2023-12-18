use std::{
	fs::File,
	sync::{
		atomic::{AtomicBool, Ordering},
		mpsc::Receiver,
		Arc, RwLock,
	},
	thread::JoinHandle,
};

use devout::{Devout, Framerate};

use crate::capture::{Frame, FRAMERATE};

pub fn start_mp4_h264_writer(
	frame: Arc<RwLock<Frame>>,
	shutdown: Arc<AtomicBool>,
	rx: Receiver<()>,
) -> JoinHandle<()> {
	std::thread::spawn(move || mp4_h264_writer(frame, shutdown, rx))
}

pub fn mp4_h264_writer(frame: Arc<RwLock<Frame>>, shutdown: Arc<AtomicBool>, rx: Receiver<()>) {
	let file = File::create("out.mp4").unwrap();
	let mut h264 = Devout::new(file, Framerate::Whole(FRAMERATE));

	loop {
		if shutdown.load(Ordering::Relaxed) {
			println!("Finishing MP4!");
			h264.done();
			println!("MP4 Finished!");
			return;
		}

		match rx.recv() {
			Err(_e) => (),
			Ok(_) => {
				println!("before read acq");
				let read = frame.read().unwrap();

				let frame_size = read.width * read.height * 3;
				h264.frame(read.width as u32, read.height as u32, &read.data);
				println!("after frame");
			}
		}
	}
}
