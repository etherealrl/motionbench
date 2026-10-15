import {
  CircleCheckIcon,
  InfoIcon,
  Loader2Icon,
  OctagonXIcon,
  TriangleAlertIcon,
} from "lucide-react";
import { Toaster as Sonner, type ToasterProps } from "sonner";
import { useDark } from "@/lib/theme";

const Toaster = ({ ...props }: ToasterProps) => {
  const dark = useDark();
  return (
    <Sonner
      theme={dark ? "dark" : "light"}
      icons={{
        success: <CircleCheckIcon className="size-4" />,
        info: <InfoIcon className="size-4" />,
        warning: <TriangleAlertIcon className="size-4" />,
        error: <OctagonXIcon className="size-4" />,
        loading: <Loader2Icon className="size-4 animate-spin" />,
      }}
      style={
        {
          "--normal-bg": "var(--color-popover)",
          "--normal-text": "var(--color-popover-foreground)",
          "--normal-border": "var(--color-border)",
          "--border-radius": "var(--radius)",
          // room for a short message and an action on one line
          "--width": "460px",
        } as React.CSSProperties
      }
      {...props}
    />
  );
};

export { Toaster };
