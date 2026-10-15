# settings

the app saves its settings in `%USERPROFILE%\.motion-bench\config.json`, the first time you change one. that's the recording button, the USB root and the mouse you picked, the target capture settings, the UI scale, the theme (light, dark, or the same as Windows), where recordings go and which sections of Capture setup are open (and the aim settings, which only a developer build uses)

a change saves itself after 300 ms, and anything still waiting gets saved when you close the window. a value outside its limits is refused and the field goes back to what was saved. if the file can't be written, the change still works until you close the app, and a notice tells you it wasn't saved

a file that can't be read gets copied aside as `config.corrupt-<n>.json` and replaced with the defaults, and a notice tells you where the copy is. a file saved by a newer version of the app is left alone, the app starts with the defaults and saves nothing

on startup it picks your mouse again once it shows up, even on another USB port, but only when exactly one mouse matches. if it's missing or two could be it, nothing is picked. restoring never starts a recording or arms anything

a new recordings folder is used from the next start, since the running app already has its library open in the old one, so the settings offer a restart (never while recording). the recordings you already made stay where they are, import them from there if you want them listed, and the app checks it can write in the new folder before saving it
