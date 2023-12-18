mod capture;
mod fluffy;
mod nv12scary;
mod videographer;

use std::{
	ops::DerefMut,
	sync::{
		atomic::{AtomicBool, Ordering},
		mpsc::channel,
		Arc,
	},
};

use fluffy::{event::Event, event_loop::ControlFlow, FluffyWindow, PhysicalSize, WindowBuilder};

fn main() {
	let wbuilder = WindowBuilder::new()
		.with_title("trichlroide")
		.with_inner_size(PhysicalSize::new(1280, 720));

	let mut fluff = FluffyWindow::build_window(wbuilder);

	let shutdown = Arc::new(AtomicBool::new(false));

	let el = fluff.take_el();
	let proxy = el.create_proxy();

	println!("Getting camera!");
	let mut camera = capture::start_camera(proxy, shutdown.clone());

	println!("Starting h264 output thread");
	let (tx, rx) = channel();
	let mut h264 = Some(videographer::start_mp4_h264_writer(
		camera.shared_frame.clone(),
		shutdown.clone(),
		rx,
	));

	el.run(move |event, elwt| {
		elwt.set_control_flow(ControlFlow::Wait);

		match event {
			Event::LoopExiting => {
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

				let mut buffer = fluff.buffer();

				// Frame received! Shove it in our buffer and redraw
				let read = camera.shared_frame.read().unwrap();
				let scaled = neam::nearest(
					read.data.as_slice(),
					3,
					read.width as u32,
					read.height as u32,
					buffer.width as u32,
					buffer.height as u32,
				);

				let frame_data = buffer.data.deref_mut();
				for (idx, pix) in scaled.chunks(3).enumerate() {
					frame_data[idx] =
						((pix[0] as u32) << 16) | ((pix[1] as u32) << 8) | (pix[2] as u32);
				}
				buffer.present();

				fluff.window.request_redraw();
			}
			_ => (),
		}

		fluff.common_events(&event, elwt);
	})
	.unwrap();
}
