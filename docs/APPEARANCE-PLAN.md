# Appearance settings plan

1. Keep the existing Light, Dark, and System modes, accent choices, and reading controls. Add built-in palettes and per-mode overrides stored with the existing device preferences, so switching modes preserves both variants.
2. Group editable colors by interface, code frame, and syntax. Show the inherited color or the current override, validate hex input, and support resetting both single colors and an entire variant. Warn when important text and code colors have low contrast.
3. Preview the active palette on a sample conversation and a real highlighted code block. Wire code colors into the workspace editor as well as the shared code renderer.
4. Add speed controls for common transitions and entrances, plus a replayable motion sample. Always respect system reduced motion, and also allow reducing motion within the app.
5. Verify persistence, switching and resetting palettes, invalid color handling, and preview interactions with frontend tests. Run typecheck and the frontend suite. Native visual review remains a manual check.

## Home wallpaper artwork

The built-in ImageGen tool generated the four images bundled under `src/assets/wallpapers/`. Each prompt ended with “Desktop wallpaper for a minimalist app. 16:9 landscape composition, no people, words, logos, watermarks, text, or UI.”

- **sakura.webp** — A wide anime background painting of a cherry blossom lane beside a quiet Japanese canal at dusk, rose and lavender tones, petals in the air, graceful branches framing the upper left and right, muted canal in center.
- **space.webp** — A wide realistic deep-space wallpaper, immense blue violet nebula stretching behind a dark rocky planet on the left, sparse stars, soft low contrast center, cinematic astrophotography.
- **ocean.webp** — A wide cinematic photographic wallpaper of a deep turquoise ocean coast from above, sweeping rolling waves and rocky shore on the right, soft mist, subtle evening sunlight.
- **mountains.webp** — A wide cinematic photographic wallpaper of layered alpine mountains and a reflective lake at sunrise, cool slate and warm gold, atmospheric mist, peaceful composition.
