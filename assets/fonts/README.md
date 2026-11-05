# Bundled font

The application embeds `NotoSansJP-Regular.ttf`, a static weight-400 instance of
[Google Fonts Noto Sans JP](https://github.com/google/fonts/tree/main/ofl/notosansjp).
Japanese and Latin text use this same family.

Upstream `NotoSansJP[wght].ttf` Git blob SHA-1:
`cdd8f083c1f5928ff3361f8cda4d3fc9462cbe89`.
Upstream OFL notice Git blob SHA-1:
`1c9f43281b8f216c5461fe9ac729afbade7724e4`.
Retrieved from the public `google/fonts` repository on 2026-11-05.

The upstream variable font's default axis is weight 100. The static instance
uses weight 400 so egui displays regular text consistently. It was produced
with `fontTools.varLib.instancer.instantiateVariableFont(font, {"wght": 400})`.
The unmodified variable source can be kept locally as `NotoSansJP.ttf` and is
excluded from Git. No fontTools dependency is needed to build or run the app.

The font remains under the SIL Open Font License 1.1. See [OFL.txt](OFL.txt).
The portable archive includes this notice as well as the embedded font in the
executable.
