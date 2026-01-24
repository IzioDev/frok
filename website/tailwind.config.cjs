/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./src/**/*.{astro,html,js,jsx,md,mdx,svelte,ts,tsx,vue}"],
  theme: {
    extend: {
      fontFamily: {
        sans: ["\"Space Grotesk\"", "system-ui", "sans-serif"],
        mono: ["\"JetBrains Mono\"", "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      colors: {
        night: "#0b0f14",
        panel: "#121826",
        line: "#1b2535",
        steel: "#8b9bb0",
        neon: "#6ef3ff",
        mint: "#8bffb0",
        pulse: "#4f7cff",
      },
      boxShadow: {
        glow: "0 30px 80px rgba(79, 124, 255, 0.25)",
      },
    },
  },
  plugins: [],
};
