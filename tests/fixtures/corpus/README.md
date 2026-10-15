# metadata corpus

these are NOT real recordings or real user files. each one holds the keys the current writer leaves
that the readers use, or is written broken on purpose, so the readers keep getting tested against
what they meet. the recording tests pair each file with `../mouse-1000hz.pcap` (bus 1, address 31,
endpoint 0x81) plus a second mouse, in a temporary folder, and check the readers leave every byte
alone.

| file | shape |
| --- | --- |
| `recording-v2/*.json` | the shape the writer leaves: complete, degraded (the control journal could not be saved), failed (device lost), interrupted by closing the app and by stopping capture |
| `marker/active.json` | the marker a recording leaves while it runs, and a crash leaves behind |
| `interrupted/recorder.incomplete.json` | what the recorder writes when the sidecar could not be published |
| `malformed/*.json` | truncated, the wrong json type, and the current shape with a null or string selection or an unknown decoder |
| `unsupported/future-recording-schema.json` | the current shape with a newer `schema` |
| `unsupported/future-version.json` | the current shape with a newer timing schema |
| `catalog/*.json` | library.json that is broken, the wrong json type, wrong field types |
| `config/*.json` | config.json that is broken, an array, an unknown version, or out of range |

the only real captures are the two pcaps one folder up; `../mouse-1000hz.json` is the metadata the
app would have written for the first one. real recordings can be checked locally with
`MOTION_BENCH_CORPUS`, see `src/testkit/corpus.rs`.
