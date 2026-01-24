// @ts-check
import { defineConfig } from "astro/config";
import tailwind from "@astrojs/tailwind";

import cloudflare from "@astrojs/cloudflare";

export default defineConfig({
  site: "https://frok.it",
  integrations: [tailwind()],
  adapter: cloudflare(),
});
