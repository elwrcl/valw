#version 300 es
precision highp float;
uniform vec2 res;
uniform float t;
uniform float motion;   // 0 flow, 1 swirl
uniform float speed;
uniform float scale;    // blob size: 1 = the spike's
uniform vec3 c1;
uniform vec3 c2;
uniform vec3 c3;
uniform sampler2D overlay;
uniform bool use_overlay;
out vec4 color;

vec2 warp(vec2 p, float time) {
    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        p += 0.42 * sin(p.yx * vec2(1.55, 1.28) + time * vec2(0.27, 0.33) + fi * vec2(1.3, 2.1));
        p = mat2(0.82, -0.57, 0.57, 0.82) * p;
    }
    return p;
}

vec3 paint(vec2 frag) {
    vec2 uv = (frag - 0.5 * res) / res.y;
    float time = t * speed;
    if (motion > 0.0) {
        float r = length(uv);
        float a = atan(uv.y, uv.x) + motion * (2.2 / (r + 0.35)) + time * 0.25 * motion;
        uv = r * vec2(cos(a), sin(a));
    }
    vec2 p = warp(uv * 4.0 / scale, time);
    p = warp(p * 1.7, time * 1.25 + 3.0);
    float v = sin(p.x) * cos(p.y);
    float w = sin(length(p) * 0.6 - time * 0.8);
    float m = v * 0.75 + w * 0.25;
    vec3 col = c1;
    col = mix(col, c2, smoothstep(-0.32, -0.18, m));
    col = mix(col, c3, smoothstep(0.38, 0.5, m) * 0.85);
    float rim = 1.0 - smoothstep(0.0, 0.06, abs(m + 0.25));
    col *= 1.0 - 0.3 * rim;
    float gloss = pow(abs(cos(p.x * 1.3 + p.y)), 36.0);
    col += gloss * 0.18 * (c3 + 0.25);
    col *= 1.0 - 0.3 * dot(uv, uv);
    return col;
}

void main() {
    vec2 frag = vec2(gl_FragCoord.x, res.y - gl_FragCoord.y);
    vec3 col = paint(frag);
    if (use_overlay) {
        vec4 o = texture(overlay, frag / res);
        col = o.rgb + col * (1.0 - o.a);
    }
    color = vec4(col, 1.0);
}
