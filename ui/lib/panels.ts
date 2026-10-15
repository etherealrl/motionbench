// the feature sets a comparison can use, in the words the interface shows
export const panelLabels: Record<string, string> = {
  texture19: "Texture (19 features)",
  trajectory14: "Trajectory (14 features)",
  full49: "Full (49 features)",
};

export const panelLabel = (panel: string) => panelLabels[panel] ?? panel;
