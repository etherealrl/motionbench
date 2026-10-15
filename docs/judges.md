# how the judges work

the judges are the ones from [ABCurves](https://github.com/optima-manent/ABCurves/blob/e197441f8d343d3b4a4069c7cae20764d0953d15/DETECTION.md), pinned at commit `e197441`, rewritten in Rust so nothing runs Python. Motion Bench has the labeled comparisons from there, so baseline against candidate. it doesn't have the CNN, the interactive target task, or the "which person is this" experiment, which needs way more people than one PC has

## the features

the app calls them Texture (19 features), Trajectory (14 features) and Full (49 features), the docs use the short names

- Texture19 cuts the movement into 160 ms windows and describes each one, the counts, how they're spread over time, the pauses and everything. it only needs the USB reports
- Trajectory14 describes each flick towards a target, its shape and how it closes in on the target, so it needs the target's position and size from [target capture](target-capture.md)
- Full49 is both of those plus the seam between the flick and the 256 ms before it

the reports get summed into 1 ms ticks first, which is what ABCurves does too. a target is never guessed from the USB capture alone, so without target capture the last two just aren't available

## W1

W1 (the Wasserstein distance) says how far apart the two sides are on each feature, measured in the baseline's own spread. it ranks which features differ the most, which is useful to see WHAT changed, though it says nothing about whether that's more than chance

## AUC

a logistic regression is trained on some recordings and asked about recordings it never saw. the AUC is how well it ranked the windows or events of those held out recordings, 0.5 being a coin flip and 1 being perfect. the split is by recording, a recording's windows or events never end up on both the training and the testing side, otherwise the judges would just learn to recognize the recording. Physical input / wire output is the exception, both of its sides are in one recording, so it splits that recording into stretches of 8 windows and a stretch never ends up on both sides

it uses up to 5 folds, repeated 5 times with seed 7, so the same recordings always give the same result

## the interval and p

the interval comes from resampling the held out recordings 200 times inside the fits that already ran. it doesn't refit anything, so it says how much the result moves with these recordings, not what a whole new set would give. it only shows when every fold had at least 2 recordings of each side, because with fewer the resampling would only shuffle the fold results around and look more precise than it is

p comes from 2,000 shuffles of which side each recording is on, refitting every time. it's two sided, so it asks how often a shuffle lands as far from 0.5 as the real AUC, in either direction. with few recordings there are only so many ways to shuffle them, which is where the floor in [comparing](comparing.md#every-recording-is-one-sample) comes from

## how close it is to ABCurves

the features are computed in f32 like the reference and the solver runs in f64. checked against the reference's own outputs, Texture19 and Trajectory14 match, and Full49 differs on one near tie in the ranking, which is about 0.001 of AUC. the tests say that tolerance out loud

## the plots

every plot reads the time range you picked, and baseline and candidate share bins and axes so they're comparable. paths and movement are in mouse counts, before Windows sensitivity. the long intervals that don't fit the main histogram are still in the overflow count, the full range plot and the markers on the timeline, nothing gets cut. a blank stretch on the timeline means no report came in then. the missing report estimates come from the usual report interval, they're a guess, not dropped reports anyone saw
