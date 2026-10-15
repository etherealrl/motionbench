import "vite/client";

// dynamic values reach CSS through custom properties, never through inline properties
declare module "react" {
  interface CSSProperties {
    [property: `--${string}`]: string | number;
  }
}
