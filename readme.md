# trichloride
trichrome-ish video because what [dslr-trichrome](https://github.com/eclecticnybles/gaze/tree/main/dslr-trichrome) gives me is fantastic.

This uh, will probably not work on your computer? It assumes the frames from the webcam are YUV422 because that's what my computer gives me even when it claims it's giving me a different format. I'm going to try to fix it, it just might take a second. Okay?

Be warned! If you compile and run it'll write an MP4 with the image you're seeing, except full res, to the current directory. It's called `out.mp4`. The H264+MP4 code is- *oh god it's so messy*. I was fighting a bug *(which turned out to be me forgetting the join the thread that was writing out the mp4)* for probably four hours? So there are little artifacts and anachronisms of me trying to fix that. I want to clean up the code and abstract the video file writing code out. Maybe make it a little more useful and, possibly, some audio stuff.
