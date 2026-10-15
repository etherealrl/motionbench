import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { native } from "@/lib/native";

/** a newer release offered at start, as a toast that stays until it's closed. a failed check says
 * nothing, since being offline isn't something you need to hear about */
export function useUpdate(onError: (message: string) => void) {
  useEffect(() => {
    if (!native) return;
    const offer = (version: string) =>
      toast.info(`Motion Bench ${version} is out`, {
        id: "update",
        description:
          "the app closes for a few seconds while it updates, your settings and recordings stay",
        duration: Infinity,
        closeButton: true,
        action: {
          label: "Update and restart",
          onClick: (event) => {
            // otherwise sonner closes the toast, and it stays to show the download
            event.preventDefault();
            toast.loading("Downloading the update…", {
              id: "update",
              description: undefined,
              action: undefined,
              closeButton: false,
            });
            // a refused or failed update is offered again. on Windows the installer ends the app, so
            // getting back here without an error means the update went away
            invoke("install_update").then(
              () => toast.dismiss("update"),
              (e: unknown) => {
                onError(String(e));
                offer(version);
              },
            );
          },
        },
      });
    void invoke<string | null>("check_update").then(
      (version) => {
        if (version) offer(version);
      },
      () => {},
    );
  }, []);
}
