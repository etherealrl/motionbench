# timing

what the timestamps in a recording are, and what they can't tell you

## the USB timestamps

every report's time comes from USBPcap, which stamps it in the kernel when Windows' USB stack sees the request complete. that's way better than stamping it when an app gets around to reading it, though it still isn't the moment the mouse sent it on the wire. the USB controller, interrupts and the driver all sit in between, and nothing on the PC side can see past them. for that you'd need a hardware USB analyzer

Motion Bench never changes those timestamps. a report stamped earlier than the one before it is kept in the raw files and left out of the decoded ones, it isn't fixed up

## the clocks

USBPcap stamps in UTC, while everything else the app times (the screen frames, the recording button, the box's input) uses the PC's performance counter. so during a recording the app reads both clocks back to back about once a second, and that's how everything gets onto one timeline. every jump of more than 1 ms between the two gets counted and kept with the recording. Recording clocks in a recording's ⋯ menu in Runs shows all of it

## the box's input

with Record physical input on, a Medius sends what it got from your mouse, stamped with its own microsecond clock. it gets lined up with the USB capture by one fixed shift for the whole recording. the app also watches how that shift moves over the recording, which shows when the alignment is drifting, though it can't tell clock drift apart from the serial link just being slower at times. a reset of the box's clock, a reconnect or lost input means the two can't be paired, and the recording says so

## display delay

the display delay is how long a mouse movement takes to show up in the frames target capture sees. it includes the game, rendering, the desktop and the detector on top of your monitor, so it's specific to your setup. Motion Bench can estimate it from a short recording:

1. in the game, look at one target that doesn't move, near the middle of the screen
2. with target capture on, record 5 to 10 seconds of moving back and forth around it, irregularly, without clicking, strafing or the target changing
3. in Capture setup, open Response delay in Target capture, pick that recording and confirm the target didn't move
4. press Estimate delay. if the fit is clear, Use for future captures sets it

it tries every delay from 0 to 250 ms and keeps the one where the target stays the most in place once your movement is taken out. the range around it is every delay that fits almost as well, it's not a confidence interval. a fit that's ambiguous, sits at the edge of the range, or had too little movement can't be applied, and estimating it with an assist on would just fit the assist, so do it with your own aim

it only changes future recordings, the ones you already have keep what they were made with

## the other observers

next to USBPcap, the app also reads Windows' Raw Input, so you can see when the same movement reached apps, in batches like Windows hands it out. just so you know, Windows merges a background app's mouse input into a packet every 8 ms or so, and Motion Bench is in the background while you play, so its Raw Input shows the same movement in fewer, bigger steps. the USB capture isn't affected. there's also an optional kernel mouse filter driver for developers, which shows when the mouse driver accepted it. normal builds only show it when it's installed, and nothing in the app needs it, the judges only ever read the USB reports
