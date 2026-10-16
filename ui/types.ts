export type Decoder = "Unavailable" | { Descriptor: unknown } | { WindowsDescriptor: unknown };
export interface Device {
  id: string;
  bus: number;
  address: number;
  endpoint: number;
  vid: number | null;
  pid: number | null;
  kind: string;
  interval: number | null;
  decoder: string;
  reports: number;
  /** what Windows calls the device behind this endpoint, when it said */
  name?: string | null;
  /** the connected box that clones this endpoint, by name, when this endpoint is its clone */
  box_name?: string | null;
}
export interface Health {
  records: number;
  reports: number;
  timestamp_regressions: number;
  serial_dropped: number;
  pairing_valid: boolean;
  ended: boolean;
  /** usb capture itself failed and has to be restarted */
  error: string | null;
  /** the selected device or a recording had a problem that does not stop capture */
  notice?: string | null;
}
/** one recording's figures for the Comparison tab's table */
export interface Numbers {
  rate_hz: number | null;
  period_us: number | null;
  jitter_us: number | null;
  longest_moving_us: number | null;
  bunched: number | null;
  beyond_1x: number | null;
  empty_ticks: number | null;
  after_gap: number | null;
  lag1_dx: number | null;
  moving: number;
}
/** the tabs over the readouts and the timeline; live, only the shown one is computed */
export type Tab = "timing" | "movement" | "relay" | "comparison";
/** axes two plots share: the timeline's band and the jitter's span either side of the period */
export interface Scale {
  band: [number, number];
  jitter_us: number;
}
export interface Readouts {
  rate_hz: number;
  /** the median moving interval */
  period_us: number | null;
  /** the median distance of a 1x moving interval from the period */
  jitter_us: number | null;
  longest_moving_us: number | null;
  median_gap_us: number | null;
  moving: number;
  gaps: number;
}
export interface Jitter {
  span_us: number;
  bins: number[];
  outside: number;
  mad_us: number | null;
  p01_us: number | null;
  p99_us: number | null;
  intervals: number;
  left_out: number;
}
export interface Density {
  columns: number;
  rows: number;
  x_max: number;
  periods: number;
  /** row major, rows from the bottom */
  cells: number[];
  /** intervals placed, those of `periods` or more, and those with no moving neighbour */
  intervals: number;
  outside: number;
  alone: number;
  after_gap: { periods: number; ratio: number; intervals: number } | null;
}
export interface Burst {
  from_us: number;
  to_us: number;
  reports: number;
  intervals: number;
  median_us: number | null;
  longest_us: number;
  rate_hz: number | null;
}
export interface Timing {
  jitter: Jitter;
  /** bunched, about 1x, 2x, 3x, and 4x or more */
  groups: [number, number, number, number, number];
  /** [threshold_us, share longer] on a log grid */
  longer: [number, number][];
  density: Density;
  /** 1 ms ticks while moving holding 0, 1, 2, 3 or more reports */
  ticks: [number, number, number, number];
  bursts: { rows: Burst[]; more: number } | null;
}
/** moving intervals whose neighbours moved at this speed, in counts per millisecond */
export interface Speed {
  from: number | null;
  to: number | null;
  intervals: number;
  median_us: number | null;
  longest_us: number | null;
}
export interface Movement {
  motion: [number, number, number, number][];
  path: [number, number][];
  /** moving reports by |dx| and |dy|: 0 to 10, then more */
  dx: number[];
  dy: number[];
  lag1_dx: number | null;
  lag1_dy: number | null;
  lag1_pairs: number;
  /** [from, to] seconds where no report came for over 50 ms */
  pauses: [number, number][];
  speed: Speed[];
}
export interface Relay {
  box_counts: [number, number];
  wire_counts: [number, number];
  /** [seconds from the view's start, box minus wire dx, dy] */
  conserved: [number, number, number][];
  /** box minus wire dx and dy per 100 ms */
  windows: [number, number][];
  matched: number;
  box_only: number;
  wire_only: number;
  offset: {
    span_us: number;
    bins: number[];
    outside: number;
    p01_us: number | null;
    p99_us: number | null;
  };
}
export interface Plot {
  from_us: number;
  to_us: number;
  reports: number;
  regressions: number;
  readouts: Readouts;
  scale: Scale;
  /** per column [shortest, longest, median, moved] */
  timeline: ([number, number, number, number] | null)[];
  timeline_start_us: number;
  timeline_step_us: number;
  tabs: {
    timing: Timing | null;
    movement: Movement | null;
    relay?: Relay | null;
  };
  movement_available: boolean;
}
export interface Config {
  seed: number;
  folds: number;
  repeats: number;
  bootstrap: number;
  permutations: number;
  width_ms: number;
}
export interface Judge {
  auc: number;
  auc_ci: [number, number] | null;
  repeat_auc: number[];
  fold_auc: number[][];
  permutation_auc: number[];
  p_value: number | null;
  /** the smallest p the shuffles could give with this many groups */
  smallest_p: number | null;
  null_p95: number | null;
  n_real: number;
  n_fake: number;
  n_groups: number;
  permutation_error: string | null;
  permutation_method: string;
}
export interface ResultRow {
  /** its own id, from its content */
  id: string;
  schema: string;
  judge_revision: string;
  implementation: { app_version: string; revision: string };
  /** the method's sampling grid, not the device's report period */
  tick_us: number;
  panel: string;
  comparison: string;
  config: Config;
  /** recording ids, each one in `sources` */
  baseline: string[];
  candidate: string[];
  sources: SourceRecord[];
  feature_names: string[];
  w1: number[];
  real_features: number[][];
  fake_features: number[][];
  judge: Judge | null;
  judge_unavailable: string | null;
  idle_dropped: [number, number];
  alignment_shift_us: number | null;
  /** the recordings it read that didn't end complete, as they were when it ran */
  not_complete: NotComplete[];
  metadata: Record<string, unknown>;
}
/** a recording a result read, and the sha256 of each file of it that was read */
export interface SourceRecord {
  id: string;
  path: string;
  digests: Record<string, string>;
}
export interface NotComplete {
  id: string;
  path: string;
  kind: RunStatus["kind"];
  message: string | null;
}
/** one box a scan saw */
export interface ListedBox {
  key: string;
  /** its family and port, such as Medius COM18 */
  label: string;
  model: string | null;
  /** it gives buttons or moves, a keyboard box doesn't */
  usable: boolean;
}

/** a port the last scan couldn't use, and anybox's reason, which names the port */
export interface UnusableBox {
  port: string;
  why: string;
}
/** the box as the window shows it, the same for every box */
export interface BoxState {
  link: "none" | "connecting" | "connected";
  error: string | null;
  pending_commands?: number;
  /** which kind of box it is, as anybox names it, such as Medius or MAKCU V4 */
  family: string;
  name: string;
  /** the serial port the box answered on, such as COM5 */
  port?: string | null;
  firmware: string;
  /** a mouse is plugged into the box */
  has_mouse?: boolean;
  boxes?: ListedBox[];
  unusable?: UnusableBox[];
  /** the connected box's key, one of `boxes` */
  key?: string | null;
  /** the box taken at the next start */
  remembered?: string | null;
  /** more than one box is plugged in and none is picked yet */
  picking?: boolean;
  notice?: string | null;
  /** why this box can't give each feature right now, null where it can */
  features: { physical_input: string | null };
  /** the box's own input is being recorded */
  watch: boolean;
  /** input the box said it lost on its way to the app */
  losses: number;
  medius: MediusState | null;
}
/** what only a Medius has: its renderer and its own counters */
export interface MediusState {
  name: string;
  port: string | null;
  mac: string;
  clone?: string;
  kind: string;
  has_mouse: boolean;
  /** whether the mouse's reports carry an id, unknown until the box says */
  report_id: boolean | null;
  rate_confident: boolean;
  firmware: {
    device_major: number;
    device_minor: number;
    device_patch: number;
    host_major: number | null;
    host_minor: number | null;
    host_patch: number | null;
    protocol: number;
  };
  native_period_us: number;
  render: { mode: string; full: boolean; ready: boolean; effective: string };
  emit: {
    mode: string;
    fixed_hz: number | null;
    resolved_hz: number;
    advertised_hz: number;
    force_hz: number | null;
    force_active: boolean;
  };
  spread: { percent: number; span_us: number };
  riding_ms: number | null;
  imperfect_allowed: boolean;
  clone_imperfect: boolean;
  health: {
    link_drops: number;
    host_drops: number;
    tx_drops: number;
    session: number;
    resets: number;
  };
}
export interface Run {
  id: string;
  path: string;
  name: string;
  /** when the recording began, null when nothing beside it says */
  started_us: number | null;
  devices: Device[];
  selected: string | null;
  decoder: Decoder;
  paired: boolean;
  panels: string[];
  missing: Record<string, string>;
  metadata: Record<string, unknown>;
  /** how the recording ended, decided natively with the same words a refused comparison uses */
  status: RunStatus;
  /** why each kind of comparison won't read it, null when it will */
  refusals: { comparison: string | null; paired: string | null };
  result: ResultRow | null;
  /** the span every analysis reads, on the pcap clock; kept in the library, never in the files */
  crop: { from_us: number; to_us: number } | null;
}
export interface RunStatus {
  kind: "complete" | "degraded" | "failed" | "interrupted";
  /** the stable code the recording wrote, when it wrote one */
  reason: string | null;
  message: string | null;
}
/** where a displayed result came from, so an export never picks up another one */
export type ResultSource = { kind: "job"; id: number } | { kind: "result"; id: string };
export interface Job {
  id: number;
  status: "running" | "complete" | "failed" | "cancelled";
  progress: number;
  error: string | null;
  result: ResultRow | null;
}
/** what the native side sends every frame. the runs aren't in it, they're read with `runs` when
 * `library` moves */
export interface Frame {
  dev_mode?: boolean;
  /** this build has the developer tools, the aim controller and its Experiment view among them */
  developer_tools?: boolean;
  /** why the last settings change was not written, until one is */
  settings_unsaved?: string | null;
  recording_hotkey?: RecordingHotkeyStatus;
  observers?: ObserverStatus;
  experiment?: ExperimentStatus;
  vision?: VisionStatus;
  seq: number;
  live: boolean;
  devices: Device[];
  selected: string | null;
  health: Health;
  recording: string | null;
  recording_elapsed_ms?: number | null;
  plot: Plot;
  /** the live view is paused on this range */
  range?: { from_us: number; to_us: number } | null;
  box: BoxState;
  /** moves with every change to the runs */
  library: number;
  job: Job | null;
  readiness: Readiness;
}
/** a frame with the runs the window lists */
export interface Snapshot extends Frame {
  runs: Run[];
}
/** the library's runs at a revision, the answer to `runs` */
export interface Listed {
  revision: number;
  runs: Run[];
}
/** what is missing, the native `Need`; its tests check every code is here */
export type Need =
  | "broken"
  | "closing"
  | "busy"
  | "library_loading"
  | "recording_button_picking"
  | "aim_button_picking"
  | "recording"
  | "armed"
  | "capture_stopped"
  | "capture_failed"
  | "no_mouse"
  | "mouse_unchecked"
  | "observer_failed"
  | "observer_starting"
  | "target_capture_stopped"
  | "target_capture_failed"
  | "target_model_warming"
  | "scenario_unnamed"
  | "model_missing"
  | "model_preparing"
  | "box_connecting"
  | "no_box"
  | "box_not_picked"
  | "box_no_mouse"
  | "box_failed"
  | "box_busy"
  | "buttons_lost"
  | "same_button"
  | "aim_failed";
/** the first thing in an action's way, decided natively; the same words a refused command uses */
export interface Blocker {
  need: Need;
  /** lowercase, shown through `sentence` */
  message: string;
  /** it passes by itself, otherwise the user has something to fix */
  waits: boolean;
}
/** each control's next action, null when it can go ahead; a control that is on asks to turn off */
export interface Readiness {
  record: Blocker | null;
  arm: Blocker | null;
  recording_button: Blocker | null;
  clear_recording_button: Blocker | null;
  aim_button: Blocker | null;
  box_connect: Blocker | null;
  box_settings: Blocker | null;
  screen_capture: Blocker | null;
}
export interface RecordingHotkeyStatus {
  hotkey_mask: number | null;
  ready: boolean;
  learning: boolean;
  held_mask: number;
  binding_seq: number;
  busy: boolean;
  message: string | null;
  error: string | null;
}
export interface ObserverStatus {
  device_changes?: number;
  raw_ready: boolean;
  kernel_ready: boolean;
  kernel_seen: boolean;
  recording: boolean;
  reports: number;
  batches: number;
  max_batch: number;
  queue_losses: number;
  kernel_losses: number;
  kernel_prior_losses: number;
  kernel_history_reports: number;
  os_loss_count_known: boolean;
  error: string | null;
  kernel_status: string;
  devices: Record<string, string>;
  kernel_devices: Record<string, string>;
}
export interface ResponseCalibration {
  selected_range_seconds?: [number, number];
  single_target_segments?: number;
  excluded_samples?: number;
  estimated_delay_ms: number;
  admissible_delay_ms: [number, number];
  residual_counts: number;
  radius_counts: number;
  movement_span_counts: number;
  frames: number;
  span_ms: number;
  identifiable: boolean;
  reason: string | null;
  residual_curve: [number, number][];
}
/** the aim's speed and deadzone, the part of its settings the ui changes */
export interface AimTuning {
  speed: number;
  deadzone_counts: number;
}
/** the mouse to select again, by what stays the same when it is plugged back in */
export interface SavedMouse {
  vid: number | null;
  pid: number | null;
  bus: number;
  endpoint: number;
}
/** which Capture setup sections are open */
export interface SetupSections {
  targets: boolean;
  box: boolean;
  box_settings: boolean;
}
/** the app's colors, or whatever the system uses */
export type Theme = "system" | "light" | "dark";
export interface DisplaySettings {
  zoom: number;
  sections: SetupSections;
  /** how loud the recording cues are, 0 to 1, kept while muted */
  volume: number;
  muted: boolean;
  theme: Theme;
}
/** the settings as native saved them; bindings and the capture choice are saved where they are made */
export interface AppSettings {
  recording: { button: number | null };
  capture: { root: string | null; mouse: SavedMouse | null };
  aim: AimTuning & { button: number };
  vision: VisionConfig;
  targets: TargetSettings;
  display: DisplaySettings;
  /** where recordings go from the next start on, null for Motion Bench in Documents */
  storage: { folder: string | null; move_from: string | null };
}
export interface TargetSettings {
  start_on_launch: boolean;
  /** your sensitivity in each game, by the game's id */
  sensitivities: Record<string, number>;
}
/** what the ui may change, each section whole */
export interface SettingsPatch {
  aim?: AimTuning;
  vision?: VisionConfig;
  targets?: TargetSettings;
  display?: DisplaySettings;
}
export interface ExperimentStatus {
  ready: boolean;
  armed: boolean;
  active: boolean;
  learning: boolean;
  hotkey_mask: number;
  held_mask: number;
  binding_seq: number;
  phase: string;
  commands: number;
  counts: [number, number];
  target_counts: [number, number] | null;
  recording: boolean;
  /** armed, with the box or the mouse behind it gone for now; it aims again once they're back */
  away?: boolean;
  error: string | null;
  /** the buttons' trouble: the feed is gone, or a learned button could not be saved */
  buttons_error?: string | null;
}
/** which way across the screen a FOV is measured, 4:3 being the width of a 4:3 screen as tall */
export type FovAxis = "horizontal" | "vertical" | "horizontal_4x3";
export interface VisionConfig {
  provider: string;
  cuda_device: number;
  cuda_graph: boolean;
  tensorrt_fp16: boolean;
  model: string;
  scenario: string;
  adapter: number;
  output: number;
  fov: number;
  fov_axis: FovAxis;
  sensitivity: number;
  yaw: number;
  pitch: number;
  confidence: number;
  target_class: number;
  fps: number;
  display_delay_ms: number;
  max_frame_gap_ms: number;
  max_residual_radius: number;
}
/** over the last ten seconds; for a rate the worst is its 1% lows, for a time its slowest 1% */
export interface Spread {
  average: number;
  worst: number;
}
export interface VisionStatus {
  fps?: number;
  capture_fps?: number;
  analyzed?: Spread | null;
  captured?: Spread | null;
  pipeline?: Spread | null;
  frame_age_ms?: number | null;
  viewport?: [number, number];
  throttled?: number;
  event_ready?: boolean;
  running: boolean;
  ready: boolean;
  frames: number;
  skipped: number;
  provider: string;
  model_shape: string;
  transport?: string;
  optimization_warning?: string | null;
  inference?: unknown;
  /** the screen's clock, what lines its frames up with the mouse */
  clock: {
    initial_bracket_us: number;
    qpc_origin: number;
    qpc_frequency: number;
    epoch_origin_us: number;
  } | null;
  recording: boolean;
  error: string | null;
  last: {
    source_us: number;
    received_us: number;
    completed_us: number;
    preprocess_ms: number;
    pipeline_ms: number;
    inference_ms: number;
    skipped: number;
    detections: {
      bounds: [number, number, number, number];
      confidence: number;
      class: number;
    }[];
  } | null;
}

/** a recording with target capture played back, the native `Replay` */
export interface Replay {
  width: number;
  height: number;
  /** how far the eye sits from the screen, in pixels */
  focal: number;
  /** degrees per count, yaw then pitch */
  degrees: [number, number];
  starting_pitch: number;
  from_us: number;
  /** the mouse's running counts at the end of each 1 ms tick */
  mouse: [number, number][];
  /** the ticks the left button went down in */
  clicks: number[];
  frames: { at_ms: number; boxes: [number, number, number, number][] }[];
}
