# trichloride
trichrome-ish video because what [dslr-trichrome](https://github.com/eclecticnybles/gaze/tree/main/dslr-trichrome) gives me is fantastic.

This uh, will probably not work on your computer? It assumes the frames from the webcam are YUV422 because that's what my computer gives me even when it claims it's giving me a different format. I'm going to try to fix it, it just might take a second. Okay?

## Other things
As well as trichloride, the tri-chrome video thing, a few other projects live here as they're video related and some are used in trichloride itself.

### `devout`
convenient crate for outputting MP4s with H264 encoded video. It aims to have a
simple API to make it as easy as possible to output video.

Currently encodes with OpenH264, but wants to eventaully use x264.

### `aisle51`
A thing for testing how fast `devout` can do things. Creates test video.

Named after the Aisle at my local Michaels that has all the picture frames.
