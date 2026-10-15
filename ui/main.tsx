import ReactDOM from "react-dom/client";
import { TooltipProvider } from "@/components/ui/tooltip";
import { loadSettings } from "@/lib/settings";
import { App } from "./App";
import "@fontsource-variable/inter";
import "./style.css";

// the saved settings come first, the scale among them
void loadSettings().finally(() =>
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <TooltipProvider>
      <App />
    </TooltipProvider>,
  ),
);
