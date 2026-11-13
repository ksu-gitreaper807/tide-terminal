# Tide identity

The terminal chevron and wave combine a command prompt with Tide's name. The wave sits in the underscore position of `>_`, with a clear gap between the two glyphs. At the 128-unit source size, the chevron's stroked right edge is 53 and the wave's stroked left edge is 63: a 10-unit gap. The wave's centerline sits at y=87, just below the chevron's lower endpoint at y=86. The favicon uses the same geometry at one-quarter scale. All SVGs are self-contained paths: no embedded bitmaps, font dependencies, scripts, filters or external resources.

| File | Use |
|---|---|
| `tide-icon.svg` | Scalable application/desktop icon, with a rounded ink tile |
| `favicon.svg` | Compact 32-unit SVG favicon; also checked at 16 pixels |
| `tide-logo.svg` | Transparent horizontal logo for light backgrounds |
| `tide-logo-dark.svg` | Transparent horizontal logo for dark backgrounds |
| `window-icon.png` | 128-pixel export for the native window API, which requires RGBA pixels |

The palette follows the terminal: ink `#14161a`, blue `#a0bce0`, sea teal `#6eb8cc` and light text `#e1e5eb`. Wordmark glyphs are outlined DejaVu Sans Bold; its license is included in [the font notices](../fonts/DejaVu-LICENSE.txt). The mark is project artwork under the repository's MIT license.

Edit the SVG sources. Regenerate the native export with librsvg's developer tool:

```bash
rsvg-convert -w 128 -h 128 -o assets/branding/window-icon.png assets/branding/tide-icon.svg
```

For a website, use `favicon.svg` directly:

```html
<link rel="icon" href="favicon.svg" type="image/svg+xml">
```

The native app embeds the pixel export for X11 and advertises the Wayland application ID `tide-terminal`. The supplied [desktop entry](../../packaging/tide-terminal.desktop) expects `tide-terminal` on PATH. Distribution packages should install that entry into `share/applications/tide-terminal.desktop` and `tide-icon.svg` into `share/icons/hicolor/scalable/apps/tide-terminal.svg`. Wayland compositors resolve the icon through that matching desktop entry; no desktop files are installed automatically when the app starts.
