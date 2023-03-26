// Ripped this from another project, unreleased

pub fn yuv422_rgb(yuv: &[u8], rgb: &mut [u8], width: usize) {
	for idx in 0..yuv.len() / 2 {
		let y = get_y(yuv, idx);
		let u = get_u(yuv, idx, width);
		let v = get_v(yuv, idx, width);

		let clr = YUV_to_RGB(y, u, v);
		rgb[idx * 3] = clr[0];
		rgb[idx * 3 + 1] = clr[1];
		rgb[idx * 3 + 2] = clr[2];
	}
}

#[inline(always)]
fn get_y(yuv: &[u8], idx: usize) -> u8 {
	yuv[idx * 2 + 1]
}

#[inline(always)]
fn get_u(yuv: &[u8], idx: usize, width: usize) -> u8 {
	let hwidth = width / 2;
	let fourth_idx = idx / 2;
	let x = fourth_idx % hwidth;
	let y = fourth_idx / hwidth;
	let u_pixel_idx = y * hwidth + x;

	yuv[u_pixel_idx * 4]
}

#[inline(always)]
fn get_v(yuv: &[u8], idx: usize, width: usize) -> u8 {
	let hwidth = width / 2;
	let fourth_idx = idx / 2;
	let x = fourth_idx % hwidth;
	let y = fourth_idx / hwidth;
	let u_pixel_idx = y * hwidth + x;

	yuv[u_pixel_idx * 4 + 2]
}

// copy/pasted from http://paulbourke.net/dataformats/nv12/
// and then fixed to work in Rust. Fixed to mean very-poorly-ported
#[allow(non_snake_case)]
fn YUV_to_RGB(y: u8, u: u8, v: u8) -> [u8; 3] {
	let u = u as f32 - 128.0;
	let v = v as f32 - 128.0;

	let r = y as f32 + 1.370705 * v;
	let g = y as f32 - 0.698001 * u - 0.337633 * v;
	let b = y as f32 + 1.732446 * u;

	let r = r.round().clamp(0.0, 255.0) as u8;
	let g = g.round().clamp(0.0, 255.0) as u8;
	let b = b.round().clamp(0.0, 255.0) as u8;

	[r, g, b]
}
