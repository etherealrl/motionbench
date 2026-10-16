# target capture

target capture watches your screen while you record, finds the targets with an ONNX model and saves where they were, next to the mouse's reports. that's what Trajectory14 and Full49 need, since a USB capture alone has no idea where the target was. it only looks at the screen, it never moves the mouse or clicks

no screen images or video get saved, only the boxes the model found and when

## what the game needs

- borderless window at your display's native resolution, with the game filling the whole screen
- one target at a time, round, the same size, and not moving
- no strafing, recoil, zoom or anything else that moves the view other than your mouse

an aim trainer scenario with static targets is the easy case. the targets are estimated from the model's boxes, not read from the game, so they're only as good as the model is

## setting it up

1. in Capture, pick your mouse and check that movement and left clicks show up
2. in Capture setup, open Target capture, pick the model with Browse, name the scenario and pick the display. a recording without a scenario name gets no target events
3. pick your game under Projection, which fills in how it turns your mouse into the camera (Valorant, CS2, Apex and Overwatch 2 for now), then type your sensitivity exactly as it is in the game's settings, and your FOV too for Apex and Overwatch 2, since those let you change it. the save button keeps that sensitivity for the game, so picking it again later brings it back
4. start target capture and wait until it's ready. a recording started before that keeps its USB data but has no target events, and so does one cut short by stopping capture or closing the app
5. record like usual. the trials get built when the recording is saved, and Runs tells you why if a recording ended up with none

the projection matters a lot. if the sensitivity or FOV doesn't match the game, the targets end up in the wrong place relative to your aim, and the judges will separate that instead of what you're testing. the replay (below) is the quick way to check

Start with the app turns target capture on when Motion Bench opens, so the model is loaded by the time you're in game

## the model

you bring the model, Motion Bench doesn't ship one. it has to be a YOLOv8 style detector exported to ONNX, self contained (weights inside the file), and Target class picks which of its classes is the target

a 320 px version runs about twice as fast as the usual 640 one, but it misses small targets. `tools/vision/resize-yolo.py` converts a YOLOv8 model without retraining it. the helpers here are in the repo and not in the installer, so grab the source (Source code (zip) on the release works) and run them from its folder:

```powershell
.venv/Scripts/python.exe -m pip install -r tools/vision/requirements-vision.txt
.venv/Scripts/python.exe tools/vision/resize-yolo.py models/v8_640_target.onnx models/v8_320_target.onnx --size 320
```

it refuses a model it doesn't recognize. test it on the smallest targets your scenario has, in a quick test a target with a 6 px radius dropped from 0.89 confidence to 0.34 at 320, under the default 0.5 cutoff

## providers

- TensorRT is the fastest, NVIDIA only
- CUDA is NVIDIA only too, and CUDA graph (on by default) keeps the frames on the GPU
- DirectML works on any DirectX 12 GPU, AMD and Intel included, with nothing else to install
- CPU works everywhere but it's slow

switching between DirectML and the NVIDIA ones needs an app restart, because they use different ONNX Runtime builds. if the provider you picked can't load, you get told why, it never quietly falls back to another one

the app comes with ONNX Runtime and DirectML. CUDA needs NVIDIA's toolkit and cuDNN installed on the system, and the helper from the source does it with the right versions (it asks for admin rights):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/vision/setup-vision.ps1 -InstallSystem
```

TensorRT goes on top of that, about 2 GB:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/vision/setup-tensorrt.ps1
```

the first time TensorRT starts with a model, it builds an engine for your GPU, which takes a few minutes. after that it's cached and starts in about a second. a recording made while it builds has no target events

on an RTX 4070 SUPER, the whole pipeline for one frame (copy, preprocessing, inference, decoding) took:

| model | DirectML | CUDA graph | TensorRT FP16 |
| ----- | -------: | ---------: | ------------: |
| 320   |  1.51 ms |    1.27 ms |       0.64 ms |
| 640   |  2.70 ms |    1.74 ms |       0.87 ms |

that's with the game paused and without grabbing the screen, so it isn't your in game FPS. the preview shows the real numbers while you play

## how trials are made

every left click ends a trial, and the trial starts with the first movement after the previous click. AimLab and probably other games show the old target and the new one together for a frame or so after a click, so anything in the first 100 ms after a click that doesn't show exactly one target is skipped

a trial needs one target the whole way, at least 3 frames of it, no gap over 50 ms between frames, and the target staying put once your mouse movement is taken out. a trial that doesn't fit is left out with the reason in `.events.json`, nothing gets guessed. where a second target stayed on screen for 50 ms or more, the crop slider in Runs shows it in red so you can cut it out

where the camera pointed up or down when you started isn't recorded, so it's worked out from the trials themselves, once per recording

event building stops at 20 minutes, 10 million reports or 300,000 frames, with an error saying so. the USB recording is still all there

## replay

Crop and replay, next to Inspect in Runs, plays it back, with the targets as circles, your crosshair as a white dot in the middle, a ring when you click, and optionally the path your crosshair took over the last few seconds. you can scrub, pause and slow it down. it's the same dialog as the crop, so moving a crop edge shows the frame it lands on, and Start here and End here set the crop where the replay is

it's drawn with the same camera math as the trials, so when the projection is right, the dot sits on the target when you click. when it's off, you'll see it right away

## display delay

the display delay shifts what the screen showed back in time, so it lines up with the mouse movement that caused it. 0 is just where it starts. [timing](timing.md#display-delay) explains how to measure yours

## the files

| file                | what's in it                                                                                                           |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `.vision.json`      | the settings, the model's hash, the provider and the pipeline that actually ran, and how it ended                      |
| `.targets.jsonl`    | every analyzed frame with its boxes and timing                                                                         |
| `.events.json`      | every trial, kept or left out, and why                                                                                 |
| `.events.npz`       | the kept trials in ABCurves' format                                                                                    |
| `.events.stale.txt` | why the last build of the events failed. while it's there, the old events are never compared, and they get built again |

a recording where target capture failed keeps its USB data, it just can't be used for Trajectory14 or Full49
