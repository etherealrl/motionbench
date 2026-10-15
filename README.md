<div align="center">

# Motion Bench

record your mouse, change one thing, and see if it can be told apart

<p>
  <img src="docs/badges/rust.svg#gh-dark-mode-only" />
  <img src="docs/badges/rust-light.svg#gh-light-mode-only" />
  <img src="docs/badges/tauri.svg#gh-dark-mode-only" />
  <img src="docs/badges/tauri-light.svg#gh-light-mode-only" />
  <br />
  <img src="docs/badges/windows.svg#gh-dark-mode-only" />
  <img src="docs/badges/windows-light.svg#gh-light-mode-only" />
  <img src="docs/badges/support.svg#gh-dark-mode-only" />
  <img src="docs/badges/support-light.svg#gh-light-mode-only" />
</p>

<img src=".github/preview-dark.png#gh-dark-mode-only" alt="Motion Bench showing a recording" width="960" />
<img src=".github/preview-light.png#gh-light-mode-only" alt="Motion Bench showing a recording" width="960" />

</div>

## what it does

Motion Bench records what your mouse actually sends over USB, so you can compare one condition against another, like your mouse plugged in directly against the same mouse through a passthrough box, two box firmwares against each other, or your own aim against a program that moves the mouse for you

it puts both sides next to each other in plots (timing, movement, paths and everything), then runs the same judges [ABCurves](https://github.com/optima-manent/ABCurves) uses. they train on some of your recordings and try to guess which side the other ones came from, so the result tells you how well the two conditions can be told apart, and how much your recordings can actually back that up

everything runs on your PC, nothing gets uploaded. every recording is kept with the settings that explain it, so you can always go back and check what you compared

## what you need

- Windows and [USBPcap](https://desowin.org/usbpcap/), which is what sees the mouse's reports
- a passthrough box if you want to test one, plugged into the PC by BOTH of its ports, the one it sends the mouse through and its COM port. anything [anybox](https://github.com/etherealrl/anybox) supports works (Medius, MAKCU, Ferrum, YSC8K), and on a Medius you also get its renderer settings and what the box saw on its side of the mouse
- target capture is optional. it watches the screen for targets with an ONNX model you bring, which the judges need for the shape of your aim on top of its texture. an NVIDIA GPU makes it fast, though DirectML and the CPU work too
- a game without an anticheat, obviously. AimLab is what we recommend, since you get one clear target and a scenario you can repeat as many times as you want, see [comparing](docs/comparing.md)

## install

grab the installer from the [latest release](https://github.com/etherealrl/motionbench/releases/latest) and run it. it isn't signed, since a certificate costs more than this project makes, so Windows SmartScreen will probably warn you about it, just click More info and then Run anyway

target capture works right away with DirectML or the CPU, since the installer comes with what those need. CUDA and TensorRT need NVIDIA's own installs on top, [target capture](docs/target-capture.md) has the commands

when a new version is out, the app tells you when it starts and Update and restart installs it, though not while you're recording. your settings and recordings stay where they are

### building it yourself

you need the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (MSVC build tools and WebView2), Rust and [Bun](https://bun.sh)

```powershell
bun install --frozen-lockfile
powershell -NoProfile -ExecutionPolicy Bypass -File tools/vision/setup-vision.ps1
bun run tauri build
```

the installer ends up in `target/release/bundle/nsis`. the second line fetches the ONNX Runtime and DirectML files the installer bundles, and nothing builds before it ran once, `cargo test` included

## usage

1. in Capture, start monitoring, move your mouse and pick it
2. press Record, do your thing, and stop. the recording shows up in Runs, where you can rename it, crop the start and end, and replay it if it has target capture
3. change the one thing you're testing and record again. do a few recordings of each side, alternating between them (A, B, A, B)
4. in Compare, put one side under baseline and the other under candidate, and run the judges

[comparing two conditions](docs/comparing.md) goes through it for each case, native against a box, box against box, the Medius renderer, and an assist program

if your mouse goes through a box, you can bind one of its buttons to start and stop recording, so you never have to tab out of the game. a tone tells you when it started and when it saved

## reading the result

the main number is the AUC. 0.5 means the judges couldn't tell the two sides apart any better than a coin flip, and 1 means they put every window of the recordings they never saw on the right side. it can land under 0.5 too, and with a large p that's just chance going the other way, so read it like 0.5

the AUC alone doesn't mean much without the p next to it, which says how likely you'd get an AUC that far from 0.5 if both sides were the same. the thing is that each recording counts as ONE sample, so with 2 recordings a side p can never go under about 0.33, however long they are. many short recordings beat a few long ones, 5 a side is a good start

a high AUC doesn't prove anything on its own and a low one doesn't mean it can't be detected, it only says how well these judges separated these recordings. before you trust an A against B result, run a control (the same condition split in two), which should land near 0.5 with a large p. [how the judges work](docs/judges.md) has the details

## the command line

`motion-bench-cli` runs Physical input / wire output on Medius recordings made with Record physical input on, without opening the app:

```powershell
cargo run --release -- judge path/to/recording.pcap
```

it reads each whole recording, so a crop you set in the app doesn't apply

## docs

- [comparing two conditions](docs/comparing.md), the rules and one walkthrough per case
- [how the judges work](docs/judges.md), what AUC, p and the interval mean
- [target capture](docs/target-capture.md), setting it up for a game or an aim trainer
- [timing](docs/timing.md), the clocks behind every timestamp and what they can't tell you
- [recordings and results](docs/recordings.md), what's in each file
- [settings](docs/settings.md), where they're saved and what restoring them does

## support

if Motion Bench saves you some time, you can [sponsor my work](https://github.com/sponsors/etherealrl), it genuinely helps

> GitHub is still approving the sponsors page, so the link might not work yet. it should be up soon

## license

[CC BY-NC-SA 4.0](LICENSE), so you can share it, change it and fork it as long as you credit ethereal, don't use it commercially, and keep your changes under the same license. the judges come from [ABCurves](https://github.com/optima-manent/ABCurves) and keep [its license](licenses/abcurves.LICENSE), and the rest of what's bundled is in [third-party notices](docs/third-party.md)

<div align="center">
  <sub>record it, compare it, don't guess</sub>
</div>
