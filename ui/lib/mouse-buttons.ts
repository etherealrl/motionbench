// a box button mask in words. a mask with more than one button is not a binding
export function buttonName(mask: number) {
  const names = ["Left", "Right", "Middle", "Side 1", "Side 2"];
  const index = Math.log2(mask);
  return Number.isInteger(index) ? (names[index] ?? `Mouse ${index + 1}`) : "Unset";
}
