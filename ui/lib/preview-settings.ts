import type { AppSettings } from "@/types";

/** the preview's settings outside the app; the app starts from what native saved */
export const previewSettings: AppSettings = {
  recording: { button: null },
  capture: { root: null, mouse: null },
  targets: { start_on_launch: false },
  aim: { speed: 2000, deadzone_counts: 12, button: 8 },
  vision: {
    provider: "CUDA",
    cuda_device: 0,
    cuda_graph: true,
    tensorrt_fp16: true,
    model: "",
    scenario: "",
    adapter: 0,
    output: 0,
    width: 2560,
    height: 1440,
    horizontal_fov: 103,
    sensitivity: 1.6,
    yaw: 0.07,
    pitch: 0.07,
    confidence: 0.5,
    target_class: 0,
    fps: 0,
    display_delay_ms: 0,
    max_frame_gap_ms: 50,
    max_residual_radius: 0.5,
  },
  display: {
    zoom: 1.25,
    sections: { targets: false, box: false, box_settings: false },
    volume: 0.6,
    muted: false,
    theme: "system",
  },
  storage: { folder: null },
};
