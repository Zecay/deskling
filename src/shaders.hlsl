// Every buddy is one instance: a quad from SV_VertexID, its data from a structured buffer.
struct Buddy { float2 pos; float unit; float sy; float angle; float frame; float flip; float eye; float3 tint; float shadow; float3 tint2; float pattern; };

cbuffer Frame : register(b0) { float2 screen; float glow; float unused; };
StructuredBuffer<Buddy> buddies : register(t0);
Texture2D<uint> atlas : register(t1);   // 0 clear, 1 body, 2 eye, 3 blush, 4 heart eye
Texture2D<float> halo : register(t2);   // a blurred silhouette of every frame
SamplerState soft : register(s0);

struct V { float4 pos : SV_Position; float2 uv : UV; nointerpolation float frame : FRAME;
           nointerpolation float3 tint : TINT; nointerpolation float flip : FLIP; nointerpolation float eye : EYE;
           nointerpolation float3 tint2 : TINTB; nointerpolation float pattern : PATTERN; };

float4 clip_at(float2 p) { return float4(p.x / screen.x * 2 - 1, 1 - p.y / screen.y * 2, 0, 1); }

V SpriteVS(uint v : SV_VertexID, uint i : SV_InstanceID) {
    Buddy b = buddies[i];
    float2 c = float2(v & 1, v >> 1);
    float sx = 1 + (1 - b.sy) * 0.8;
    float2 size = float2(16 * sx, 10 * b.sy) * b.unit;
    float2 p = (c - 0.5) * size;
    float ca = cos(b.angle), sa = sin(b.angle);
    float2 turned = float2(ca * p.x - sa * p.y, sa * p.x + ca * p.y);
    V o;
    o.pos = clip_at(b.pos + float2(0, -size.y / 2) + turned);
    o.uv = c; o.frame = b.frame; o.tint = b.tint; o.flip = b.flip; o.eye = b.eye; o.tint2 = b.tint2; o.pattern = b.pattern;
    return o;
}

float4 SpritePS(V i) : SV_Target {
    int x = min(15, int(i.uv.x * 16));
    int y = min(9, int(i.uv.y * 10));
    if (i.flip > 0.5) x = 15 - x;
    uint cell = atlas.Load(int3(int(i.frame) * 16 + x, y, 0));
    if (cell == 0) discard;
    if (cell == 1) {   // the same rule as the Studio preview
        int p = int(i.pattern);
        if (p == 1) return float4(lerp(i.tint, i.tint2, saturate((y - 2) / 6.0)), 1);
        if (p == 2 && (y % 2) == 1) return float4(i.tint2, 1);
        if (p == 3 && ((x + 2 * y) % 5) == 0) return float4(i.tint2, 1);
        return float4(i.tint, 1);
    }
    if (cell == 2) return float4(i.eye, i.eye, i.eye, 1);
    if (cell == 3) return float4(0.957, 0.635, 0.604, 1);
    return float4(1, 0.302, 0.427, 1);
}

V GlowVS(uint v : SV_VertexID, uint i : SV_InstanceID) {
    Buddy b = buddies[i];
    float2 c = float2(v & 1, v >> 1);
    float sx = 1 + (1 - b.sy) * 0.8;
    float2 p = (c - 0.5) * float2(24 * sx, 18 * b.sy) * b.unit;
    float ca = cos(b.angle), sa = sin(b.angle);
    float2 turned = float2(ca * p.x - sa * p.y, sa * p.x + ca * p.y);
    V o;
    o.pos = clip_at(b.pos + float2(0, -5 * b.sy * b.unit) + turned);
    o.uv = c; o.frame = b.frame; o.tint = b.tint; o.flip = b.flip; o.eye = 0; o.tint2 = b.tint2; o.pattern = 0;
    return o;
}

float4 GlowPS(V i) : SV_Target {
    float x = i.flip > 0.5 ? 1 - i.uv.x : i.uv.x;
    float a = halo.Sample(soft, float2((i.frame + x) / 26.0, i.uv.y));
    a = a * a * (3 - 2 * a) * glow;            // eases out to nothing, with no visible edge
    float3 light = lerp(i.tint, float3(1, 1, 1), 0.28);
    return float4(light * a, a * 0.45);        // more colour than cover, so it reads as light
}

V ShadowVS(uint v : SV_VertexID, uint i : SV_InstanceID) {
    Buddy b = buddies[i];
    float2 c = float2(v & 1, v >> 1);
    V o;
    o.pos = clip_at(b.pos + (c - 0.5) * float2(15, 3.5) * b.unit * b.shadow);
    o.uv = c; o.frame = 0; o.tint = 0; o.flip = 0; o.eye = 0; o.tint2 = 0; o.pattern = 0;
    return o;
}

float4 ShadowPS(V i) : SV_Target {
    float a = saturate(1 - length((i.uv - 0.5) * 2));
    return float4(0, 0, 0, 0.32 * a * a);   // premultiplied
}
