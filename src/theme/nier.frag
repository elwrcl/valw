#version 300 es
precision highp float;
uniform vec2 res;
uniform float t;
uniform float motion;   // widens the scan band (0 calm, 1 wide)
uniform float speed;
uniform float scale;    // grid spacing: 1 = 28 px
uniform vec3 c1;        // fill
uniform vec3 c2;        // grid and squares
uniform vec3 c3;        // scan band
uniform sampler2D overlay;
uniform bool use_overlay;
out vec4 color;

// NieR-menu backdrop: dot grid, a few drifting squares, a slow scan band,
// grain and a vignette. No warp loops: a handful of hashes per pixel.

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

vec3 nier(vec2 frag) {
    float time = t * speed;
    vec3 col = c1;

    // 2 px dots every 28 px
    float cell = max(8.0, floor(28.0 * scale + 0.5));
    vec2 g = mod(frag, cell);
    col = mix(col, c2, step(g.x, 2.0) * step(g.y, 2.0) * 0.10);

    // 14 squares, 3–9 px, each drifting its own way and wrapping
    for (int i = 0; i < 14; i++) {
        float fi = float(i);
        float h1 = hash(vec2(fi, 3.1));
        float h2 = hash(vec2(fi, 7.9));
        float size = 3.0 + floor(h1 * 7.0);
        vec2 dir = normalize(vec2(h1 - 0.5, h2 - 0.5) + 0.001);
        vec2 p = mod(vec2(fract(h1 * 13.7), fract(h2 * 7.3)) * res + dir * time * 18.0, res);
        vec2 d = abs(frag - p);
        float sq = step(max(d.x, d.y), size * 0.5);
        col = mix(col, c2, sq * (0.15 + 0.2 * fract(h2 * 5.1)));
    }

    // a thin band sweeping down every 9 s (at speed 1)
    float y = fract(time / 9.0) * (res.y + 40.0) - 20.0;
    float width = 2.0 + 6.0 * motion;
    col = mix(col, c3, (1.0 - smoothstep(0.0, width, abs(frag.y - y))) * 0.08);

    // grain, reseeded 12 times a second
    col += (hash(frag + floor(t * 12.0)) - 0.5) * 0.05;

    vec2 uv = (frag - 0.5 * res) / res.y;
    col *= 1.0 - 0.25 * dot(uv, uv);
    return col;
}

void main() {
    vec2 frag = vec2(gl_FragCoord.x, res.y - gl_FragCoord.y);
    vec3 col = nier(frag);
    if (use_overlay) {
        vec4 o = texture(overlay, frag / res);
        col = o.rgb + col * (1.0 - o.a);
    }
    color = vec4(col, 1.0);
}
