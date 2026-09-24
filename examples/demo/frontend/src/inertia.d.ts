// Types generated from Rust (`cargo test`, see src/views in the Loco app) wired into Inertia,
// so `usePage().props` and `usePage().flash` know the shared props and flash data.
import type { AppFlash } from "./types/AppFlash";
import type { AppShared } from "./types/AppShared";

declare module "@inertiajs/core" {
  export interface InertiaConfig {
    sharedPageProps: AppShared;
    flashDataType: AppFlash;
  }
}
