pub trait Effect {
	fn frame_in(&mut self, data: Vec<u8>, width: usize, height: usize);
	fn frame_out(&self) -> Frame;
}

pub struct Frame<'a> {
	data: &'a [u8],
	width: usize,
	height: usize,
}
