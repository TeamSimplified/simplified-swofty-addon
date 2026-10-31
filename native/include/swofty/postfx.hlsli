// Swofty post-process pass contract (postfx v1). The launcher compiles a pass as a cs_6_0 compute
// shader with this header on the include path, and the core runs it on the game's picture after the
// tonemap and before the HUD. A pass defines `float4 effect(float2 uv, uint2 pixel)` and returns the
// colour for that pixel.

#ifndef SWOFTY_POSTFX_HLSLI
#define SWOFTY_POSTFX_HLSLI

#define SWOFTY_POSTFX_SDR 0
#define SWOFTY_POSTFX_HDR10 1
#define SWOFTY_POSTFX_SCRGB 2

cbuffer SwoftyPostFx : register(b0)
{
    uint2 swofty_size;
    float2 swofty_texel;
    float swofty_time;
    uint swofty_frame;
    uint swofty_encoding;
    uint swofty_pass;
    float4 swofty_options[4];
    uint2 swofty_depth_size;
    uint swofty_has_depth;
};

Texture2D<float4> swofty_scene : register(t0);
Texture2D<float> swofty_depth : register(t1);
RWTexture2D<float4> swofty_output : register(u0);
SamplerState swofty_linear : register(s0);
SamplerState swofty_point : register(s1);

float option(uint i)
{
    return swofty_options[i >> 2][i & 3];
}

float4 scene(float2 uv)
{
    return swofty_scene.SampleLevel(swofty_linear, uv, 0);
}

float4 scene_at(int2 pixel)
{
    return swofty_scene.Load(int3(clamp(pixel, int2(0, 0), int2(swofty_size) - 1), 0));
}

// The game's depth buffer at uv, as stored: 1 at the near plane, falling towards 0 in the distance.
// 0 everywhere when the depth buffer is not available (has_depth() is false).
float scene_depth(float2 uv)
{
    return swofty_depth.SampleLevel(swofty_point, uv, 0);
}

bool has_depth()
{
    return swofty_has_depth != 0;
}

float3 srgb_to_linear(float3 c)
{
    return select(c <= 0.04045, c / 12.92, pow((c + 0.055) / 1.055, 2.4));
}

float3 linear_to_srgb(float3 c)
{
    c = max(c, 0);
    return select(c <= 0.0031308, c * 12.92, 1.055 * pow(c, 1.0 / 2.4) - 0.055);
}

float3 pq_to_linear(float3 c)
{
    const float m1 = 0.1593017578125, m2 = 78.84375, c1 = 0.8359375, c2 = 18.8515625, c3 = 18.6875;
    float3 p = pow(max(c, 0), 1.0 / m2);
    return pow(max(p - c1, 0) / (c2 - c3 * p), 1.0 / m1) * 10000.0 / 80.0;
}

float3 linear_to_pq(float3 c)
{
    const float m1 = 0.1593017578125, m2 = 78.84375, c1 = 0.8359375, c2 = 18.8515625, c3 = 18.6875;
    float3 y = pow(max(c * 80.0 / 10000.0, 0), m1);
    return pow((c1 + c2 * y) / (1 + c3 * y), m2);
}

// The picture in linear light: 1 is SDR white in SDR and 80 nits in HDR.
float3 to_linear(float3 c)
{
    if (swofty_encoding == SWOFTY_POSTFX_HDR10)
        return pq_to_linear(c);
    if (swofty_encoding == SWOFTY_POSTFX_SCRGB)
        return c;
    return srgb_to_linear(c);
}

float3 from_linear(float3 c)
{
    if (swofty_encoding == SWOFTY_POSTFX_HDR10)
        return linear_to_pq(c);
    if (swofty_encoding == SWOFTY_POSTFX_SCRGB)
        return c;
    return linear_to_srgb(c);
}

float luminance(float3 linear_rgb)
{
    return dot(linear_rgb, float3(0.2126, 0.7152, 0.0722));
}

float4 effect(float2 uv, uint2 pixel);

[numthreads(8, 8, 1)] void main(uint3 id : SV_DispatchThreadID)
{
    if (any(id.xy >= swofty_size))
        return;
    float2 uv = (float2(id.xy) + 0.5) * swofty_texel;
    swofty_output[id.xy] = effect(uv, id.xy);
}

#endif
