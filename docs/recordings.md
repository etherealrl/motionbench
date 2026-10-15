# recordings and results

recordings go to `Documents/Motion Bench` unless you pick another folder in the settings (⚙, then Change), and every file of one recording shares the same name with its start time in it. keep them together, the app needs the whole set

## the files

| file               | what's in it                                                                                                                             |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `.pcap`            | the selected mouse's USB traffic exactly as USBPcap saw it, plus its descriptors. other devices aren't in it                             |
| `.json`            | the mouse, the decoder, the box and its firmware, the clocks, and how the recording ended. written once, at the end, even when it failed |
| `.csv`             | every report with its timestamp, its interval and its raw bytes                                                                          |
| `.motion.csv`      | the decoded X and Y counts                                                                                                               |
| `.input.csv`       | what a Medius got from your mouse, with Record physical input on                                                                         |
| `.clocks.jsonl`    | the clock readings that line everything up, see [timing](timing.md)                                                                      |
| `.raw-input.jsonl` | the same mouse as Windows' Raw Input saw it, other mice plugged in stay out of it                                                        |
| `.active.json`     | only there while recording. one left behind without a `.json` means the recording never finished                                         |
| `.incomplete.json` | why a recording failed, when even its `.json` couldn't be written                                                                        |

target capture adds its own files, they're in [target capture](target-capture.md#the-files). `.motion.csv` and `.events.npz` use ABCurves' names for their columns and arrays, so its scripts read them too

## how a recording ended

the `.json` says it in `outcome`:

- `complete` means everything worked
- `degraded` means the USB data is whole but something next to it failed, like the box's input losing a few reports. it can still be compared, just not for what failed
- `failed` means the recording itself broke (the mouse got unplugged, USBPcap stopped etc), with the reason in `reason`. it's listed in Runs but never compared
- `interrupted` means stopping capture or closing the app cut it short. its files were saved, so Texture19 can still compare it, though its target events can't be used since they need the whole recording

a recording is never silently fixed or thrown away, a failed one stays in Runs so you can see what happened

## the library

Import, at the top of the app, adds a Motion Bench recording from somewhere else, by its `.pcap` with the rest of its files next to it. the names, crops and picked mice live in `library.json` in the recordings folder, so renaming a recording never touches its files. on startup the app reads it and looks through the folder, so a moved folder keeps everything, and a recording you removed from Runs stays removed until you import it again. removing one from Runs leaves its files on disk

## crops

a crop is the part of a recording every analysis reads, the plots, the comparisons, the target events and the display delay. it's saved in the library next to the name, so the files themselves never change. a trial only counts when all of it is inside the crop

the files for outside tools are never cropped, and neither is `motion-bench-cli`, since it doesn't read the library

## results

every comparison you run is saved in `results` in the recordings folder, and shows up again on its recordings after a restart. a result keeps everything needed to check it later, the recordings it read with a hash of each file, their crops, the settings, the app version and the ABCurves revision, the features of every window or trial, and the folds. once a recording's crop changes, results made with the old crop aren't shown for it anymore. Export analysis in Compare saves the same thing wherever you want

## ABCurves event files

`.events.npz` has the trials in ABCurves' array names, so the future movement, its mask, the target's position and radius at B80, the 256 ms before it, and which recording each trial came from. it's plain NumPy, no pickle
