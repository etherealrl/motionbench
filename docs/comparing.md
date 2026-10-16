# comparing two conditions

this is how to set up a comparison so the result actually means something. the rules first, because they're the same for every case, then one section per thing you might want to test

## the rules

### every recording is one sample

the judges train on some recordings and test on the others, never on two parts of the same one, so a 10 minute recording counts exactly as much as a 30 second one. the p value has a floor that only depends on how many recordings you have:

| recordings a side | lowest p you can get |
| :---------------: | :------------------: |
|         2         |      about 0.33      |
|         3         |         0.1          |
|         4         |      about 0.03      |
|         5         |     about 0.008      |

so many short recordings beat a few long ones. 5 a side of a minute or two each is a good start, and the app tells you when you have too few for a p at all

### alternate the conditions

record A, B, A, B and not A, A, A, B, B, B. you get a little tired, you warm up, the game heats up the GPU, and if all of one side comes first the judges can pick up on that instead of the thing you're testing

### record in AimLab

never use this in a game with an anticheat, a box or an assist program can get you banned there. AimLab is what we recommend, because you get one clear target at a time and a scenario that plays the same way every time, which is what makes two sides comparable. Gridshot 5x5 single target is a good one to start with

### change ONE thing

sensitivity, FOV, the game and scenario, the mouse, its DPI and polling rate, the mousepad, the box settings, the assist settings, all of it stays the same on both sides. if two things change you can't know which one the judges saw

### run a control first

record one condition 10 times and compare the 1st, 3rd, 5th, 7th and 9th against the other 5, so the two halves alternate like above. it should land near 0.5 with a large p. if it doesn't, something other than your condition is changing between recordings, and no A against B result means much until you find what

### crop the edges

the start and end of a recording are usually you tabbing in and out of the game. crop them in Runs (Crop, next to Inspect) so the judges only read the part you care about. with target capture on, the crop plays the recording back too, so you can see where the game actually starts and ends

## picking the mode and the features

in Compare, the comparison mode is almost always Separate recordings, so baseline recordings on one side and candidate recordings on the other. the two others are for specific cases:

- Physical input / wire output compares what a Medius got from your mouse against what it sent to the PC, in the same recording. it needs Record physical input on in the box's section of Capture setup before you record, and it answers "what does the box do to my movement", nothing else
- Matching source events is for event datasets where the same events show up on both sides, like one set of trials before and after some transform. you won't need it for ordinary recordings

the features decide what the judges look at. the app calls them Texture (19 features), Trajectory (14 features) and Full (49 features), and these docs use the short names:

- Texture19 reads the texture of the movement, so the counts and their timing. it works on every recording
- Trajectory14 reads the shape of each flick towards a target, and needs [target capture](target-capture.md)
- Full49 is both of those plus the movement right before each flick. it also needs target capture, and it's the one to use when something aims for you, because that's where an assist shows

## native against a box

this tells you whether going through a passthrough box changes anything the judges can see

1. plug the mouse straight into the PC and record your normal movement, 5 times or more
2. plug the mouse into the box and BOTH of the box's ports into this PC, the one it sends the mouse through (that's what gets recorded) and its COM port (that's how the app talks to it). start capture again and pick the box's output (moving the mouse helps you find it), and record the same way
3. alternate between the two setups, even if replugging is annoying, because that's what the rules above are about
4. compare with Separate recordings and Texture19, the direct recordings as baseline

the box's own report timing usually shows up in the Timing tab even when the judges can't tell the sides apart, so look at both

## one box against another

the same thing as above, with both sides on a box. keep the mouse, the cable and the USB port the same, and if the boxes run different firmware, say so in the recording names so you can still tell later

## Medius against Medius with the renderer

this tests the renderer on its own, so both sides go through the same Medius and only the renderer changes

1. connect the box's control port, so its section shows up in Capture setup. check it cloned your mouse and not your keyboard
2. for the renderer side, pick the render mode, turn on Render physical movement and apply. move the mouse for a bit so the box learns its profile, until the Relay only badge goes away. the profile lives in the box's RAM, so it's gone after a power cycle and has to be learned again
3. for the other side, turn the renderer off and keep every other setting the same (emit pace, spread, riding, the forced rate and everything)
4. record and alternate as usual, then compare with Separate recordings and Texture19

if you want to know what the box does to your movement rather than whether it can be told apart, use Physical input / wire output on the renderer recordings instead

## native or a box against an assist program

this is the one where target capture matters the most, because an assist acts on targets, and Texture19 alone can miss it

1. everything runs on this one PC, since the app has to record the box's output and see the screen at the same time. if the program is usually set up on a second PC, run it in 1PC mode instead, most of them can point at localhost or something like that
2. the box's COM port can only be open in one program at a time and the program needs it, so press Disconnect in the box's section of Capture setup (or start the program first, then Motion Bench just says the port is taken). Motion Bench doesn't need the box for this comparison, the USB capture and target capture are enough. the recording button goes through the box though, so you start and stop from the app and crop the edges. do it the same way on the side without the program too, so being connected to the box isn't a second thing that changes
3. set up [target capture](target-capture.md) for your game and check the trials come out (Runs shows why a recording has none)
4. the game's sensitivity and FOV have to match what's in target capture, on BOTH sides. if they don't, the trials are wrong on one side and the judges will happily separate that instead
5. keep the program's settings the same for every candidate recording, and write them in the recording names
6. play the same scenario with and without the program, alternating, 5 times or more each
7. compare with Separate recordings and Full49

replay a recording or two from Runs before you compare (Crop and replay, next to Inspect). the crosshair should sit on the target when you click, if it doesn't the projection settings are off

## reading the result

- AUC is how well the judges ranked the windows or events of recordings they never saw. 0.5 is a coin flip, 1 is every one of them on the right side. a recording with more windows weighs more in it, while the p counts each recording once
- p is how likely an AUC that far from 0.5 would be if both sides were the same. it can't go under the floor in the table above
- the interval around the AUC only shows when each fold had enough recordings of each side to resample. with few recordings you won't get one, and that's on purpose

an AUC under 0.5 isn't a bug. the judges are scored on recordings they never saw, so with few of them they can end up wrong more often than right just by chance, and with a large p you read it like 0.5. far under 0.5 with a small p isn't chance though, the sides do differ and the judges got the direction backwards, which usually means something else changed between your recordings

a high AUC means these judges could tell these recordings apart, not that something is detected everywhere. a low one doesn't mean it can't be detected either, just that these judges with this many recordings couldn't. and the numbers aren't comparable with a tool that splits one long session into blocks and tests on blocks of the same session, because there the judges also learn to recognize the session itself, so its AUCs come out higher. Motion Bench holds out whole recordings (stretches of one, in Physical input / wire output, since both of its sides are in the same recording)

[how the judges work](judges.md) has the details
