// language: metal2.0
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size2;
};

typedef uint ShaderBool;
typedef uint BackgroundTag;
typedef uint ColorSpace;
typedef uint BorderStyle;
typedef uint SurfaceColorFormat;
typedef uint BlurCompositeClip;
typedef uint DownsampleMode;
struct GlobalUniforms {
    metal::float2 viewport_size;
    ShaderBool premultiplied_alpha;
    uint padding;
};
struct FontRasterizationUniforms {
    metal::float4 gamma_ratios;
    float grayscale_enhanced_contrast;
    float subpixel_enhanced_contrast;
    ShaderBool uses_blue_green_red_subpixel_order;
    uint padding;
};
struct Bounds {
    metal::float2 origin;
    metal::float2 size;
};
struct Edges {
    float top;
    float right;
    float bottom;
    float left;
};
struct ContentMask {
    Bounds bounds;
    Edges fade_out;
};
struct Corners {
    float top_left;
    float top_right;
    float bottom_right;
    float bottom_left;
};
struct Hsla {
    float h;
    float s;
    float l;
    float a;
};
struct LinearColorStop {
    Hsla color;
    float percentage;
};
struct type_7 {
    LinearColorStop inner[2];
};
struct Background {
    BackgroundTag tag;
    ColorSpace color_space;
    Hsla solid;
    float gradient_angle_or_pattern_height;
    type_7 colors;
    uint padding;
};
struct AtlasTextureId {
    uint index;
    uint kind;
};
struct AtlasBounds {
    metal::int2 origin;
    metal::int2 size;
};
struct AtlasTile {
    AtlasTextureId texture_id;
    uint tile_id;
    uint padding;
    AtlasBounds bounds;
};
struct TransformationMatrix {
    metal::float2x2 rotation_scale;
    metal::float2 translation;
};
struct RectangleVertex {
    metal::float2 unit_position;
    metal::float2 viewport_position;
    metal::float4 clip_position;
};
struct FullscreenVertex {
    metal::float2 texture_coordinates;
    char _pad1[8];
    metal::float4 clip_position;
};
struct Paint {
    Background background;
    Bounds bounds;
};
struct PreparedPaint {
    metal::float4 solid;
    metal::float4 color0_;
    metal::float4 color1_;
};
struct SignedDistanceSample {
    float distance;
    char _pad1[4];
    metal::float2 normal;
    float path_t;
    uint segment;
};
struct FigmaCornerLayout {
    metal::float4 horizontal_budgets;
    metal::float4 vertical_budgets;
};
struct FigmaCornerExtents {
    metal::float4 horizontal;
    metal::float4 vertical;
};
struct type_12 {
    float inner[4];
};
struct type_13 {
    uint inner[4];
};
struct FigmaAxisParams {
    float a;
    float b;
    float p;
};
struct FigmaCornerParams {
    float radius;
    float smoothing;
    float arc_sweep;
    float c;
    float d;
    FigmaAxisParams horizontal;
    FigmaAxisParams vertical;
};
struct CubicClosestPoint {
    metal::float2 point;
    metal::float2 tangent;
    float distance;
    float path_t;
};
struct FigmaRectangleSample {
    SignedDistanceSample signed_distance;
    uint corner;
    char _pad2[4];
};
struct PreparedCorners {
    metal::float4 horizontal_reaches;
    metal::float4 vertical_reaches;
    metal::float4 smoothing_factors;
    float superellipse_power;
    char _pad4[12];
};
struct Shadow {
    uint order;
    float blur_radius;
    Bounds bounds;
    Corners corner_radii;
    ContentMask content_mask;
    Background color;
    Bounds element_bounds;
    Corners element_corner_radii;
    ShaderBool inset;
    float corner_smoothing;
};
typedef Shadow type_14[1];
struct ShadowVertexData {
    metal::float4 position;
    PreparedPaint paint;
    uint shadow_id;
    char _pad3[12];
    metal::float4 clip_distances;
};
struct ShadowVarying {
    metal::float4 position;
    metal::float4 paint_solid;
    metal::float4 paint_color0_;
    metal::float4 paint_color1_;
    uint shadow_id;
    char _pad5[12];
    metal::float4 clip_distances;
};
struct SmoothedShadowVarying {
    metal::float4 position;
    metal::float4 paint_solid;
    metal::float4 paint_color0_;
    metal::float4 paint_color1_;
    uint shadow_id;
    char _pad5[12];
    metal::float4 clip_distances;
    metal::float4 horizontal_corner_reaches;
    metal::float4 vertical_corner_reaches;
    metal::float4 element_horizontal_corner_reaches;
    metal::float4 element_vertical_corner_reaches;
    metal::float4 smoothing_factors;
};
constant float PI = 3.1415927;
constant float HALF_TURN_DEGREES = 180.0;
constant float FULL_TURN_DEGREES = 360.0;
constant float CSS_GRADIENT_OFFSET_DEGREES = 90.0;
constant float PIXEL_ANTIALIAS_RADIUS = 0.5;
constant float GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS = 3.0;
constant float PATTERN_COMPONENT_SCALE = 255.0;
constant float PATTERN_PACKING_RADIX = 65535.0;
constant float MIN_NORMALIZED_WEIGHT = 0.00001;
constant float MIN_PATH_GRADIENT = 0.001;
constant float UNDERLINE_WAVE_FREQUENCY = 2.0;
constant float UNDERLINE_WAVE_HEIGHT_RATIO = 0.8;
constant float GRADIENT_DITHER_SCALE = 0.618034;
constant float GRADIENT_DITHER_RGB_STRENGTH = 0.007843138;
constant float GRADIENT_DITHER_ALPHA_STRENGTH = 0.011764706;
constant metal::float2 GRADIENT_DITHER_SEED_A = metal::float2(12.9898, 78.233);
constant metal::float2 GRADIENT_DITHER_SEED_B = metal::float2(39.346, 11.135);
constant float GRADIENT_DITHER_MULTIPLIER_A = 43758.547;
constant float GRADIENT_DITHER_MULTIPLIER_B = 24634.635;
constant float DIAGONAL_STRIPE_ANGLE = 0.7853982;
constant metal::float3 REC_601_LUMA_WEIGHTS = metal::float3(0.3, 0.59, 0.11);
constant metal::float3 LINEAR_RGB_LUMA_WEIGHTS = metal::float3(0.2126, 0.7152, 0.0722);
constant float SRGB_DECODE_CUTOFF = 0.04045;
constant float SRGB_ENCODE_CUTOFF = 0.0031308;
constant float SRGB_TRANSFER_OFFSET = 0.055;
constant float SRGB_TRANSFER_SCALE = 1.055;
constant float SRGB_DECODE_EXPONENT = 2.4;
constant float SRGB_LINEAR_SCALE = 12.92;
constant float ERROR_FUNCTION_LINEAR_COEFFICIENT = 0.278393;
constant float ERROR_FUNCTION_QUADRATIC_COEFFICIENT = 0.230389;
constant float ERROR_FUNCTION_CUBIC_COEFFICIENT = 0.000972;
constant float ERROR_FUNCTION_QUARTIC_COEFFICIENT = 0.078108;
constant float DARK_TEXT_CONTRAST_SCALE = 4.0;
constant float DARK_TEXT_BRIGHTNESS_CUTOFF = 0.75;
constant metal::float3x3 LINEAR_SRGB_TO_CONE_RESPONSE = metal::float3x3(metal::float3(0.41222146, 0.2119035, 0.08830246), metal::float3(0.53633255, 0.6806995, 0.28171885), metal::float3(0.051445995, 0.10739696, 0.6299787));
constant metal::float3x3 CONE_RESPONSE_TO_OKLAB = metal::float3x3(metal::float3(0.21045426, 1.9779985, 0.025904037), metal::float3(0.7936178, -2.4285922, 0.78277177), metal::float3(-0.004072047, 0.4505939, -0.80867577));
constant metal::float3x3 OKLAB_TO_CONE_RESPONSE = metal::float3x3(metal::float3(1.0, 1.0, 1.0), metal::float3(0.39633778, -0.105561346, -0.08948418), metal::float3(0.21580376, -0.06385417, -1.2914855));
constant metal::float3x3 CONE_RESPONSE_TO_LINEAR_SRGB = metal::float3x3(metal::float3(4.0767417, -1.268438, -0.0041960864), metal::float3(-3.3077116, 2.6097574, -0.7034186), metal::float3(0.23096994, -0.34131938, 1.7076147));
constant uint ShaderBool_Disabled = 0u;
constant uint ShaderBool_Enabled = 1u;
constant uint BackgroundTag_Solid = 0u;
constant uint BackgroundTag_LinearGradient = 1u;
constant uint BackgroundTag_PatternSlash = 2u;
constant uint BackgroundTag_Checkerboard = 3u;
constant uint ColorSpace_Srgb = 0u;
constant uint ColorSpace_Oklab = 1u;
constant uint BorderStyle_Solid = 0u;
constant uint BorderStyle_Dashed = 1u;
constant uint SurfaceColorFormat_Rgba = 0u;
constant uint SurfaceColorFormat_Yuv = 1u;
constant uint BlurCompositeClip_None = 0u;
constant uint BlurCompositeClip_RoundedBounds = 1u;
constant uint DownsampleMode_Copy = 0u;
constant uint DownsampleMode_HalfResolution = 1u;
constant float SUPERELLIPSE_DIAGONAL_INSET = 0.29289323;
constant uint FIGMA_SEGMENT_STRAIGHT = 0u;
constant uint FIGMA_SEGMENT_FIRST_CUBIC = 1u;
constant uint FIGMA_SEGMENT_ARC = 2u;
constant uint FIGMA_SEGMENT_SECOND_CUBIC = 3u;
constant uint FIGMA_NO_CORNER = 4u;
constant float FIGMA_EPSILON = 0.000001;
constant int SHADOW_INTEGRATION_SAMPLE_COUNT = 4;

float ContentMask_alpha(
    ContentMask content_mask,
    metal::float2 position
) {
    float alpha = 1.0;
    Bounds bounds_13 = content_mask.bounds;
    Edges fade = content_mask.fade_out;
    if (content_mask.fade_out.left > 0.0) {
        float _e9 = alpha;
        alpha = _e9 * metal::saturate((position.x - content_mask.bounds.origin.x) / content_mask.fade_out.left);
    }
    if (content_mask.fade_out.right > 0.0) {
        float _e21 = alpha;
        alpha = _e21 * metal::saturate(((content_mask.bounds.origin.x + content_mask.bounds.size.x) - position.x) / content_mask.fade_out.right);
    }
    if (content_mask.fade_out.top > 0.0) {
        float _e36 = alpha;
        alpha = _e36 * metal::saturate((position.y - content_mask.bounds.origin.y) / content_mask.fade_out.top);
    }
    if (content_mask.fade_out.bottom > 0.0) {
        float _e48 = alpha;
        alpha = _e48 * metal::saturate(((content_mask.bounds.origin.y + content_mask.bounds.size.y) - position.y) / content_mask.fade_out.bottom);
    }
    float _e60 = alpha;
    return _e60;
}

metal::float2 Bounds_position(
    Bounds bounds,
    metal::float2 unit_position
) {
    return bounds.origin + (unit_position * bounds.size);
}

metal::float2 Bounds_1half_size(
    Bounds bounds_1
) {
    return bounds_1.size / metal::float2(2.0);
}

metal::float2 Bounds_center(
    Bounds bounds_2
) {
    metal::float2 _e2 = Bounds_1half_size(bounds_2);
    return bounds_2.origin + _e2;
}

bool Corners_1is_zero(
    Corners corners
) {
    bool local = {};
    bool local_1 = {};
    bool local_2 = {};
    if (corners.top_left == 0.0) {
        local = corners.top_right == 0.0;
    } else {
        local = false;
    }
    bool _e10 = local;
    if (_e10) {
        local_1 = corners.bottom_right == 0.0;
    } else {
        local_1 = false;
    }
    bool _e17 = local_1;
    if (_e17) {
        local_2 = corners.bottom_left == 0.0;
    } else {
        local_2 = false;
    }
    bool _e24 = local_2;
    return _e24;
}

bool Edges_1is_zero(
    Edges edges
) {
    bool local_3 = {};
    bool local_4 = {};
    bool local_5 = {};
    if (edges.top == 0.0) {
        local_3 = edges.right == 0.0;
    } else {
        local_3 = false;
    }
    bool _e10 = local_3;
    if (_e10) {
        local_4 = edges.bottom == 0.0;
    } else {
        local_4 = false;
    }
    bool _e17 = local_4;
    if (_e17) {
        local_5 = edges.left == 0.0;
    } else {
        local_5 = false;
    }
    bool _e24 = local_5;
    return _e24;
}

metal::float2 TransformationMatrix_1transform_position(
    TransformationMatrix transform,
    metal::float2 position_1
) {
    return (metal::transpose(transform.rotation_scale) * position_1) + transform.translation;
}

bool is_enabled(
    ShaderBool value
) {
    return value == ShaderBool_Enabled;
}

metal::float4 transparent(
) {
    return metal::float4(0.0, 0.0, 0.0, 0.0);
}

float antialiased_coverage(
    float signed_distance
) {
    return metal::saturate(PIXEL_ANTIALIAS_RADIUS - signed_distance);
}

metal::float4 premultiply(
    metal::float4 color,
    float coverage
) {
    float alpha_5 = color.w * coverage;
    return metal::float4(color.x * alpha_5, color.y * alpha_5, color.z * alpha_5, alpha_5);
}

metal::float2 atlas_texture_coordinates(
    metal::float2 unit_position_1,
    AtlasTile tile,
    metal::uint2 atlas_size
) {
    metal::float2 tile_origin = metal::float2(static_cast<float>(tile.bounds.origin.x), static_cast<float>(tile.bounds.origin.y));
    metal::float2 tile_size = metal::float2(static_cast<float>(tile.bounds.size.x), static_cast<float>(tile.bounds.size.y));
    metal::float2 texture_size = metal::float2(static_cast<float>(atlas_size.x), static_cast<float>(atlas_size.y));
    return (tile_origin + (unit_position_1 * tile_size)) / texture_size;
}

float color_brightness(
    metal::float3 color_1
) {
    return metal::dot(color_1, REC_601_LUMA_WEIGHTS);
}

float light_on_dark_contrast(
    float enhanced_contrast,
    metal::float3 color_2
) {
    float _e4 = color_brightness(color_2);
    float darkness = metal::saturate(DARK_TEXT_CONTRAST_SCALE * (DARK_TEXT_BRIGHTNESS_CUTOFF - _e4));
    return enhanced_contrast * darkness;
}

float _2apply_alpha_correction_f32_(
    float alpha_1,
    float brightness,
    metal::float4 gamma_ratios
) {
    float brightness_adjustment = (brightness * gamma_ratios.x) + gamma_ratios.y;
    float correction = (brightness_adjustment * alpha_1) + ((brightness * gamma_ratios.z) + gamma_ratios.w);
    return alpha_1 + ((alpha_1 * (1.0 - alpha_1)) * correction);
}

float _1enhance_contrast_f32_(
    float alpha_2,
    float contrast
) {
    return (alpha_2 * (contrast + 1.0)) / ((alpha_2 * contrast) + 1.0);
}

float apply_contrast_and_gamma_correction(
    float sample,
    metal::float3 color_3,
    float enhanced_contrast_factor,
    metal::float4 gamma_ratios_1
) {
    float _e4 = light_on_dark_contrast(enhanced_contrast_factor, color_3);
    float _e5 = _1enhance_contrast_f32_(sample, _e4);
    float _e6 = color_brightness(color_3);
    float _e7 = _2apply_alpha_correction_f32_(_e5, _e6, gamma_ratios_1);
    return _e7;
}

metal::float3 _1enhance_contrast_vec3f(
    metal::float3 alpha_3,
    float contrast_1
) {
    return (alpha_3 * (contrast_1 + 1.0)) / ((alpha_3 * contrast_1) + metal::float3(1.0));
}

metal::float3 _2apply_alpha_correction_vec3f(
    metal::float3 alpha_4,
    metal::float3 brightness_1,
    metal::float4 gamma_ratios_2
) {
    metal::float3 brightness_adjustment_1 = (brightness_1 * gamma_ratios_2.x) + metal::float3(gamma_ratios_2.y);
    metal::float3 correction_1 = (brightness_adjustment_1 * alpha_4) + ((brightness_1 * gamma_ratios_2.z) + metal::float3(gamma_ratios_2.w));
    return alpha_4 + ((alpha_4 * (metal::float3(1.0) - alpha_4)) * correction_1);
}

metal::float3 apply_contrast_and_gamma_correction3_(
    metal::float3 sample_1,
    metal::float3 color_4,
    float enhanced_contrast_factor_1,
    metal::float4 gamma_ratios_3
) {
    float _e4 = light_on_dark_contrast(enhanced_contrast_factor_1, color_4);
    metal::float3 _e5 = _1enhance_contrast_vec3f(sample_1, _e4);
    metal::float3 _e6 = _2apply_alpha_correction_vec3f(_e5, color_4, gamma_ratios_3);
    return _e6;
}

metal::float2 rectangle_corner(
    uint vertex_id_2
) {
    return metal::float2(static_cast<float>(vertex_id_2 & 1u), 0.5 * static_cast<float>(vertex_id_2 & 2u));
}

metal::float4 viewport_to_clip_position(
    metal::float2 position_2,
    constant GlobalUniforms& GLOBALS
) {
    metal::float2 _e3 = GLOBALS.viewport_size;
    metal::float2 clip_position = ((position_2 / _e3) * metal::float2(2.0, -2.0)) + metal::float2(-1.0, 1.0);
    return metal::float4(clip_position.x, clip_position.y, 0.0, 1.0);
}

RectangleVertex rectangle_vertex(
    uint vertex_id_3,
    Bounds bounds_3,
    constant GlobalUniforms& GLOBALS
) {
    metal::float2 _e2 = rectangle_corner(vertex_id_3);
    metal::float2 _e3 = Bounds_position(bounds_3, _e2);
    metal::float4 _e4 = viewport_to_clip_position(_e3, GLOBALS);
    return RectangleVertex {_e2, _e3, _e4};
}

RectangleVertex transformed_rectangle_vertex(
    uint vertex_id_4,
    Bounds bounds_4,
    TransformationMatrix transform_1,
    constant GlobalUniforms& GLOBALS
) {
    metal::float2 _e3 = rectangle_corner(vertex_id_4);
    metal::float2 _e4 = Bounds_position(bounds_4, _e3);
    metal::float2 _e5 = TransformationMatrix_1transform_position(transform_1, _e4);
    metal::float4 _e6 = viewport_to_clip_position(_e5, GLOBALS);
    return RectangleVertex {_e3, _e5, _e6};
}

FullscreenVertex fullscreen_vertex(
    uint vertex_id_5
) {
    metal::float2 texture_coordinates = metal::float2(static_cast<float>((vertex_id_5 << 1u) & 2u), static_cast<float>(vertex_id_5 & 2u));
    return FullscreenVertex {texture_coordinates, {}, metal::float4((texture_coordinates.x * 2.0) - 1.0, 1.0 - (texture_coordinates.y * 2.0), 0.0, 1.0)};
}

metal::float4 clip_distances(
    metal::float2 position_3,
    Bounds clip_bounds
) {
    metal::float2 distance_from_top_left = position_3 - clip_bounds.origin;
    metal::float2 distance_from_bottom_right = (clip_bounds.origin + clip_bounds.size) - position_3;
    return metal::float4(distance_from_top_left.x, distance_from_bottom_right.x, distance_from_top_left.y, distance_from_bottom_right.y);
}

bool is_clipped(
    metal::float4 distances
) {
    return metal::min(metal::min(distances.x, distances.y), metal::min(distances.z, distances.w)) < 0.0;
}

metal::float4 unclipped_distances(
) {
    return metal::float4(1.0, 1.0, 1.0, 1.0);
}

metal::float3 srgb_to_linear(
    metal::float3 srgb
) {
    metal::bool3 cutoff = metal::bool3(srgb.x < SRGB_DECODE_CUTOFF, srgb.y < SRGB_DECODE_CUTOFF, srgb.z < SRGB_DECODE_CUTOFF);
    metal::float3 higher = metal::pow((srgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4, 2.4, 2.4));
    return metal::select(higher, srgb / metal::float3(12.92), cutoff);
}

metal::float3 linear_to_srgb(
    metal::float3 linear
) {
    metal::bool3 cutoff_1 = metal::bool3(linear.x < SRGB_ENCODE_CUTOFF, linear.y < SRGB_ENCODE_CUTOFF, linear.z < SRGB_ENCODE_CUTOFF);
    metal::float3 higher_1 = (SRGB_TRANSFER_SCALE * metal::pow(linear, metal::float3(0.41666666, 0.41666666, 0.41666666))) - metal::float3(0.055);
    return metal::select(higher_1, linear * SRGB_LINEAR_SCALE, cutoff_1);
}

metal::float4 linear_to_srgba(
    metal::float4 color_5
) {
    metal::float3 _e2 = linear_to_srgb(color_5.xyz);
    return metal::float4(_e2.x, _e2.y, _e2.z, color_5.w);
}

metal::float4 srgba_to_linear(
    metal::float4 color_6
) {
    metal::float3 _e2 = srgb_to_linear(color_6.xyz);
    return metal::float4(_e2.x, _e2.y, _e2.z, color_6.w);
}

metal::float3 hue_to_unit_rgb(
    float hue
) {
    metal::float3 phases = metal::fract(metal::float3(hue, hue + 0.6666667, hue + 0.33333334));
    return metal::saturate(metal::abs((phases * 6.0) - metal::float3(3.0)) - metal::float3(1.0));
}

metal::float4 hsla_to_rgba(
    Hsla hsla
) {
    float chroma = hsla.s * (1.0 - metal::abs((2.0 * hsla.l) - 1.0));
    metal::float3 _e12 = hue_to_unit_rgb(hsla.h);
    metal::float3 red_green_blue = ((_e12 - metal::float3(0.5)) * chroma) + metal::float3(hsla.l);
    return metal::float4(red_green_blue.x, red_green_blue.y, red_green_blue.z, hsla.a);
}

metal::float4 linear_srgb_to_oklab(
    metal::float4 color_7
) {
    metal::float3 linear_srgb = metal::pow(color_7.xyz, metal::float3(2.2, 2.2, 2.2));
    metal::float3 cone_response = LINEAR_SRGB_TO_CONE_RESPONSE * linear_srgb;
    metal::float3 cube_root = metal::float3(0.33333334, 0.33333334, 0.33333334);
    metal::float3 oklab = CONE_RESPONSE_TO_OKLAB * metal::pow(cone_response, cube_root);
    return metal::float4(oklab.x, oklab.y, oklab.z, color_7.w);
}

metal::float4 oklab_to_linear_srgb(
    metal::float4 color_8
) {
    metal::float3 cone_root = OKLAB_TO_CONE_RESPONSE * color_8.xyz;
    metal::float3 cone_response_1 = (cone_root * cone_root) * cone_root;
    metal::float3 linear_srgb_1 = CONE_RESPONSE_TO_LINEAR_SRGB * cone_response_1;
    metal::float3 srgb_1 = metal::pow(linear_srgb_1, metal::float3(0.45454547, 0.45454547, 0.45454547));
    return metal::float4(srgb_1.x, srgb_1.y, srgb_1.z, color_8.w);
}

metal::float4 over(
    metal::float4 below,
    metal::float4 above
) {
    float alpha_6 = above.w + (below.w * (1.0 - above.w));
    if (alpha_6 == 0.0) {
        metal::float4 _e11 = transparent();
        return _e11;
    }
    metal::float3 red_green_blue_1 = ((above.xyz * above.w) + ((below.xyz * below.w) * (1.0 - above.w))) / metal::float3(alpha_6);
    return metal::float4(red_green_blue_1.x, red_green_blue_1.y, red_green_blue_1.z, alpha_6);
}

float gaussian(
    float position_4,
    float standard_deviation
) {
    float variance = standard_deviation * standard_deviation;
    return metal::exp(-((position_4 * position_4)) / (2.0 * variance)) / (2.5066283 * standard_deviation);
}

metal::float2 approximate_error_function(
    metal::float2 value_1
) {
    metal::float2 signs = metal::sign(value_1);
    metal::float2 absolute_value = metal::abs(value_1);
    metal::float2 polynomial = metal::float2(1.0) + (absolute_value * (metal::float2(0.278393) + (absolute_value * (metal::float2(0.230389) + (absolute_value * (metal::float2(0.000972) + (absolute_value * ERROR_FUNCTION_QUARTIC_COEFFICIENT)))))));
    metal::float2 polynomial_squared = polynomial * polynomial;
    return signs - (signs / (polynomial_squared * polynomial_squared));
}

float integrated_rounded_rectangle_coverage(
    float horizontal_position,
    float vertical_position,
    float standard_deviation_1,
    float corner_radius,
    metal::float2 half_size
) {
    float distance_beyond_corner = metal::min((half_size.y - corner_radius) - metal::abs(vertical_position), 0.0);
    float curved_extent = (half_size.x - corner_radius) + metal::sqrt(metal::max(0.0, (corner_radius * corner_radius) - (distance_beyond_corner * distance_beyond_corner)));
    metal::float2 integration_bounds = metal::float2(horizontal_position) + metal::float2(-(curved_extent), curved_extent);
    metal::float2 normalized_bounds = integration_bounds * (0.70710677 / standard_deviation_1);
    metal::float2 _e27 = approximate_error_function(normalized_bounds);
    metal::float2 integral = metal::float2(0.5) + (0.5 * _e27);
    return integral.y - integral.x;
}

float pick_corner_radius(
    metal::float2 center_to_point,
    Corners radii
) {
    float left = (center_to_point.y < 0.0) ? radii.top_left : radii.bottom_left;
    float right = (center_to_point.y < 0.0) ? radii.top_right : radii.bottom_right;
    return (center_to_point.x < 0.0) ? left : right;
}

float rounded_rectangle_signed_distance_from_corner(
    metal::float2 corner_center_to_point,
    float corner_radius_1
) {
    if (corner_radius_1 == 0.0) {
        return metal::max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        return (metal::length(metal::max(metal::float2(0.0, 0.0), corner_center_to_point)) + metal::min(0.0, metal::max(corner_center_to_point.x, corner_center_to_point.y))) - corner_radius_1;
    }
}

float rounded_rectangle_signed_distance(
    metal::float2 point,
    Bounds bounds_5,
    Corners corner_radii
) {
    metal::float2 _e3 = Bounds_1half_size(bounds_5);
    metal::float2 _e4 = Bounds_center(bounds_5);
    metal::float2 center_to_point_1 = point - _e4;
    float _e6 = pick_corner_radius(center_to_point_1, corner_radii);
    float _e11 = rounded_rectangle_signed_distance_from_corner((metal::abs(center_to_point_1) - _e3) + metal::float2(_e6), _e6);
    return _e11;
}

float gaussian_signed_distance_coverage(
    float distance,
    float standard_deviation_2
) {
    float normalized = distance / (1.4142135 * standard_deviation_2);
    metal::float2 _e6 = approximate_error_function(metal::float2(normalized, normalized));
    return metal::saturate(0.5 - (0.5 * _e6.x));
}

metal::float4 blend_color(
    metal::float4 color_9,
    float alpha_factor,
    constant GlobalUniforms& GLOBALS
) {
    float alpha_7 = color_9.w * alpha_factor;
    ShaderBool _e6 = GLOBALS.premultiplied_alpha;
    bool _e7 = is_enabled(_e6);
    float multiplier = _e7 ? alpha_7 : 1.0;
    return metal::float4(color_9.x * multiplier, color_9.y * multiplier, color_9.z * multiplier, alpha_7);
}

Paint Paint_new(
    Background background,
    Bounds bounds_6
) {
    return Paint {background, bounds_6};
}

PreparedPaint PreparedPaint_new(
    metal::float4 solid,
    metal::float4 color0_,
    metal::float4 color1_
) {
    return PreparedPaint {solid, color0_, color1_};
}

PreparedPaint prepare_paint(
    Paint paint
) {
    PreparedPaint prepared = {};
    metal::float4 _e1 = transparent();
    metal::float4 _e2 = transparent();
    metal::float4 _e3 = transparent();
    PreparedPaint _e4 = PreparedPaint_new(_e1, _e2, _e3);
    prepared = _e4;
    if (paint.background.tag == BackgroundTag_LinearGradient) {
        metal::float4 _e15 = hsla_to_rgba(paint.background.colors.inner[0].color);
        prepared.color0_ = _e15;
        metal::float4 _e21 = hsla_to_rgba(paint.background.colors.inner[1].color);
        prepared.color1_ = _e21;
        if (paint.background.color_space == ColorSpace_Srgb) {
            metal::float4 _e28 = prepared.color0_;
            metal::float4 _e29 = linear_to_srgba(_e28);
            prepared.color0_ = _e29;
            metal::float4 _e32 = prepared.color1_;
            metal::float4 _e33 = linear_to_srgba(_e32);
            prepared.color1_ = _e33;
        } else {
            metal::float4 _e36 = prepared.color0_;
            metal::float4 _e37 = linear_srgb_to_oklab(_e36);
            prepared.color0_ = _e37;
            metal::float4 _e40 = prepared.color1_;
            metal::float4 _e41 = linear_srgb_to_oklab(_e40);
            prepared.color1_ = _e41;
        }
    } else {
        metal::float4 _e45 = hsla_to_rgba(paint.background.solid);
        prepared.solid = _e45;
    }
    PreparedPaint _e46 = prepared;
    return _e46;
}

float linear_gradient_ratio(
    Paint paint_1,
    metal::float2 position_5
) {
    metal::float2 direction = {};
    float ratio = {};
    float radians = ((metal::fmod(paint_1.background.gradient_angle_or_pattern_height, FULL_TURN_DEGREES) - CSS_GRADIENT_OFFSET_DEGREES) * PI) / HALF_TURN_DEGREES;
    direction = metal::float2(metal::cos(radians), metal::sin(radians));
    if (paint_1.bounds.size.x > paint_1.bounds.size.y) {
        float _e24 = direction.y;
        direction.y = _e24 * (paint_1.bounds.size.y / paint_1.bounds.size.x);
    } else {
        float _e34 = direction.x;
        direction.x = _e34 * (paint_1.bounds.size.x / paint_1.bounds.size.y);
    }
    metal::float2 _e44 = Bounds_1half_size(paint_1.bounds);
    metal::float2 _e46 = Bounds_center(paint_1.bounds);
    metal::float2 _e48 = direction;
    metal::float2 _e50 = direction;
    ratio = metal::dot(position_5 - _e46, _e48) / metal::length(_e50);
    float _e55 = direction.x;
    float _e58 = direction.y;
    if (metal::abs(_e55) > metal::abs(_e58)) {
        float _e61 = ratio;
        ratio = (_e61 + _e44.x) / paint_1.bounds.size.x;
    } else {
        float _e68 = ratio;
        ratio = (_e68 + _e44.y) / paint_1.bounds.size.y;
    }
    float first_stop = paint_1.background.colors.inner[0].percentage;
    float last_stop = paint_1.background.colors.inner[1].percentage;
    float _e83 = ratio;
    return metal::saturate((_e83 - first_stop) / (last_stop - first_stop));
}

metal::float4 gradient_dither(
    metal::float2 position_6
) {
    metal::float2 seed = position_6 * GRADIENT_DITHER_SCALE;
    float noise_a = metal::fract(metal::sin(metal::dot(seed, GRADIENT_DITHER_SEED_A)) * GRADIENT_DITHER_MULTIPLIER_A);
    float noise_b = metal::fract(metal::sin(metal::dot(seed, GRADIENT_DITHER_SEED_B)) * GRADIENT_DITHER_MULTIPLIER_B);
    float triangular_noise = (noise_a + noise_b) - 1.0;
    return metal::float4(triangular_noise * GRADIENT_DITHER_RGB_STRENGTH, triangular_noise * GRADIENT_DITHER_RGB_STRENGTH, triangular_noise * GRADIENT_DITHER_RGB_STRENGTH, triangular_noise * GRADIENT_DITHER_ALPHA_STRENGTH);
}

metal::float4 linear_gradient_color(
    Paint paint_2,
    metal::float2 position_7,
    PreparedPaint prepared_1
) {
    metal::float4 color_10 = {};
    float _e3 = linear_gradient_ratio(paint_2, position_7);
    metal::float4 ratio4_ = metal::float4(_e3, _e3, _e3, _e3);
    metal::float4 interpolated = metal::mix(prepared_1.color0_, prepared_1.color1_, ratio4_);
    metal::float4 _e8 = transparent();
    color_10 = _e8;
    if (paint_2.background.color_space == ColorSpace_Oklab) {
        metal::float4 _e14 = oklab_to_linear_srgb(interpolated);
        color_10 = _e14;
    } else {
        metal::float4 _e15 = srgba_to_linear(interpolated);
        color_10 = _e15;
    }
    metal::float4 _e16 = color_10;
    metal::float4 _e17 = gradient_dither(position_7);
    return _e16 + _e17;
}

metal::float4 slash_pattern_color(
    Paint paint_3,
    metal::float2 position_8,
    metal::float4 solid_1
) {
    metal::float4 color_11 = {};
    float encoded = paint_3.background.gradient_angle_or_pattern_height;
    float pattern_width = (encoded / PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
    float pattern_interval = metal::fmod(encoded, PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
    float pattern_height = pattern_width + pattern_interval;
    float period_1 = pattern_height * 0.70710677;
    metal::float2x2 rotation = metal::float2x2(metal::float2(0.70710677, -0.70710677), metal::float2(0.70710677, 0.70710677));
    float pattern = metal::fmod((rotation * (position_8 - paint_3.bounds.origin)).x, period_1);
    float distance_3 = metal::min(pattern, period_1 - pattern) - ((period_1 * (pattern_width / pattern_height)) / 2.0);
    color_11 = solid_1;
    float _e38 = color_11.w;
    float _e39 = antialiased_coverage(distance_3);
    color_11.w = _e38 * _e39;
    metal::float4 _e41 = color_11;
    return _e41;
}

metal::float4 checkerboard_color(
    Paint paint_4,
    metal::float2 position_9,
    metal::float4 solid_2
) {
    metal::float4 color_12 = {};
    metal::float2 relative = position_9 - paint_4.bounds.origin;
    float square_size = paint_4.background.gradient_angle_or_pattern_height;
    float colored = metal::fmod(metal::floor(relative.x / square_size) + metal::floor(relative.y / square_size), 2.0);
    color_12 = solid_2;
    float _e19 = color_12.w;
    color_12.w = _e19 * metal::saturate(colored);
    metal::float4 _e22 = color_12;
    return _e22;
}

metal::float4 paint_color(
    Paint paint_5,
    metal::float2 position_10,
    PreparedPaint prepared_2
) {
    metal::float4 color_13 = {};
    color_13 = prepared_2.solid;
    switch(paint_5.background.tag) {
        case 0u: {
            color_13 = prepared_2.solid;
            break;
        }
        case 1u: {
            metal::float4 _e8 = linear_gradient_color(paint_5, position_10, prepared_2);
            color_13 = _e8;
            break;
        }
        case 2u: {
            metal::float4 _e10 = slash_pattern_color(paint_5, position_10, prepared_2.solid);
            color_13 = _e10;
            break;
        }
        case 3u: {
            metal::float4 _e12 = checkerboard_color(paint_5, position_10, prepared_2.solid);
            color_13 = _e12;
            break;
        }
        default: {
            break;
        }
    }
    metal::float4 _e13 = color_13;
    return _e13;
}

float corner_dash_velocity(
    float first,
    float second
) {
    if (first == 0.0) {
        return second;
    } else {
        if (second == 0.0) {
            return first;
        } else {
            return metal::min(first, second);
        }
    }
}

float signed_modulo(
    float value_2,
    float modulus
) {
    return value_2 - (modulus * metal::trunc(value_2 / modulus));
}

float dash_coverage(
    float position_11,
    float period,
    float dash_length,
    float dash_velocity
) {
    float half_period = period / 2.0;
    float half_dash_length = dash_length / 2.0;
    float _e10 = signed_modulo((position_11 + half_period) - half_dash_length, period);
    float centered = _e10 - half_period;
    float signed_distance_1 = metal::abs(centered) - half_dash_length;
    float _e15 = antialiased_coverage(signed_distance_1 / dash_velocity);
    return _e15;
}

float quarter_ellipse_signed_distance(
    metal::float2 point_1,
    metal::float2 radii_1
) {
    return ((metal::length(point_1 / radii_1) - 1.0) * (radii_1.x + radii_1.y)) * -0.5;
}

metal::float4 corner_values(
    Corners corner_radii_1
) {
    return metal::float4(corner_radii_1.top_left, corner_radii_1.top_right, corner_radii_1.bottom_right, corner_radii_1.bottom_left);
}

float normalized_superellipse_power(
    float corner_smoothing
) {
    float smoothing_1 = metal::clamp(corner_smoothing, 0.0, 1.0);
    float normalized_diagonal = 1.0 - (SUPERELLIPSE_DIAGONAL_INSET / (1.0 + smoothing_1));
    return -0.6931472 / metal::log(normalized_diagonal);
}

metal::float4 normalized_superellipse_reaches(
    Corners corner_radii_2,
    float corner_smoothing_1
) {
    metal::float4 _e2 = corner_values(corner_radii_2);
    return metal::max(_e2, metal::float4(0.0, 0.0, 0.0, 0.0)) * (1.0 + metal::clamp(corner_smoothing_1, 0.0, 1.0));
}

bool can_use_normalized_superellipse(
    metal::float2 size,
    Corners corner_radii_3,
    float corner_smoothing_2
) {
    bool local_6 = {};
    bool local_7 = {};
    bool local_8 = {};
    bool local_9 = {};
    bool local_10 = {};
    bool local_11 = {};
    bool local_12 = {};
    bool local_13 = {};
    bool local_14 = {};
    bool local_15 = {};
    metal::float4 _e3 = corner_values(corner_radii_3);
    metal::float4 radii_4 = metal::max(_e3, metal::float4(0.0, 0.0, 0.0, 0.0));
    metal::float4 _e10 = normalized_superellipse_reaches(corner_radii_3, corner_smoothing_2);
    if (corner_smoothing_2 > 0.0) {
        local_6 = size.x > 0.0;
    } else {
        local_6 = false;
    }
    bool _e19 = local_6;
    if (_e19) {
        local_7 = size.y > 0.0;
    } else {
        local_7 = false;
    }
    bool _e26 = local_7;
    if (_e26) {
        if (!((radii_4.x > 0.0))) {
            local_9 = radii_4.y > 0.0;
        } else {
            local_9 = true;
        }
        bool _e39 = local_9;
        if (!(_e39)) {
            local_10 = radii_4.z > 0.0;
        } else {
            local_10 = true;
        }
        bool _e47 = local_10;
        if (!(_e47)) {
            local_11 = radii_4.w > 0.0;
        } else {
            local_11 = true;
        }
        bool _e55 = local_11;
        local_8 = _e55;
    } else {
        local_8 = false;
    }
    bool _e57 = local_8;
    if (_e57) {
        local_12 = (_e10.x + _e10.y) <= size.x;
    } else {
        local_12 = false;
    }
    bool _e66 = local_12;
    if (_e66) {
        local_13 = (_e10.y + _e10.z) <= size.y;
    } else {
        local_13 = false;
    }
    bool _e75 = local_13;
    if (_e75) {
        local_14 = (_e10.z + _e10.w) <= size.x;
    } else {
        local_14 = false;
    }
    bool _e84 = local_14;
    if (_e84) {
        local_15 = (_e10.w + _e10.x) <= size.y;
    } else {
        local_15 = false;
    }
    bool _e93 = local_15;
    return _e93;
}

float normalized_superellipse_signed_distance_from_corner(
    metal::float2 corner_to_point,
    float corner_radius_2,
    float corner_smoothing_3,
    float power
) {
    bool local_16 = {};
    bool local_17 = {};
    float extent = metal::max(corner_radius_2, 0.0) * (1.0 + metal::clamp(corner_smoothing_3, 0.0, 1.0));
    metal::float2 corner_center_to_point_1 = corner_to_point + metal::float2(extent);
    if (!((extent <= 0.0))) {
        local_16 = corner_center_to_point_1.x <= 0.0;
    } else {
        local_16 = true;
    }
    bool _e23 = local_16;
    if (!(_e23)) {
        local_17 = corner_center_to_point_1.y <= 0.0;
    } else {
        local_17 = true;
    }
    bool _e31 = local_17;
    if (_e31) {
        return metal::max(corner_to_point.x, corner_to_point.y);
    }
    metal::float2 normalized_1 = corner_center_to_point_1 / metal::float2(extent);
    metal::float2 powered = metal::pow(normalized_1, metal::float2(power, power));
    float gradient = power * metal::length(metal::pow(normalized_1, metal::float2(power - 1.0, power - 1.0)));
    return (extent * ((powered.x + powered.y) - 1.0)) / metal::max(gradient, FIGMA_EPSILON);
}

bool can_use_compact_corner_selection(
    metal::float2 size_1,
    metal::float4 reaches
) {
    bool local_18 = {};
    bool local_19 = {};
    bool local_20 = {};
    float half_short_side = 0.5 * metal::min(size_1.x, size_1.y);
    if (reaches.x <= half_short_side) {
        local_18 = reaches.y <= half_short_side;
    } else {
        local_18 = false;
    }
    bool _e14 = local_18;
    if (_e14) {
        local_19 = reaches.z <= half_short_side;
    } else {
        local_19 = false;
    }
    bool _e20 = local_19;
    if (_e20) {
        local_20 = reaches.w <= half_short_side;
    } else {
        local_20 = false;
    }
    bool _e26 = local_20;
    return _e26;
}

float compact_normalized_superellipse_signed_distance(
    metal::float2 point_2,
    Bounds bounds_7,
    Corners corner_radii_4,
    float corner_smoothing_4,
    float power_1
) {
    metal::float2 _e5 = Bounds_1half_size(bounds_7);
    metal::float2 _e6 = Bounds_center(bounds_7);
    metal::float2 center_to_point_2 = point_2 - _e6;
    float _e8 = pick_corner_radius(center_to_point_2, corner_radii_4);
    metal::float2 corner_to_point_2 = metal::abs(center_to_point_2) - _e5;
    float _e11 = normalized_superellipse_signed_distance_from_corner(corner_to_point_2, _e8, corner_smoothing_4, power_1);
    return _e11;
}

float figma_signed_distance(
    metal::float2 delta,
    metal::float2 normal,
    float distance_1
) {
    return (metal::dot(delta, normal) >= 0.0) ? distance_1 : -(distance_1);
}

SignedDistanceSample figma_nearest_straight_sample(
    metal::float2 point_3,
    metal::float2 size_2,
    metal::float4 horizontal_extents,
    metal::float4 vertical_extents
) {
    metal::float2 nearest_delta = {};
    metal::float2 nearest_normal = metal::float2(0.0, -1.0);
    float nearest_distance_squared = {};
    metal::float2 candidate_delta = {};
    float candidate_distance_squared = {};
    nearest_delta = point_3 - metal::float2(metal::clamp(point_3.x, horizontal_extents.x, metal::max(horizontal_extents.x, size_2.x - horizontal_extents.y)), 0.0);
    metal::float2 _e20 = nearest_delta;
    metal::float2 _e21 = nearest_delta;
    nearest_distance_squared = metal::dot(_e20, _e21);
    candidate_delta = point_3 - metal::float2(size_2.x, metal::clamp(point_3.y, vertical_extents.y, metal::max(vertical_extents.y, size_2.y - vertical_extents.z)));
    metal::float2 _e36 = candidate_delta;
    metal::float2 _e37 = candidate_delta;
    candidate_distance_squared = metal::dot(_e36, _e37);
    float _e40 = candidate_distance_squared;
    float _e41 = nearest_distance_squared;
    if (_e40 < _e41) {
        metal::float2 _e43 = candidate_delta;
        nearest_delta = _e43;
        nearest_normal = metal::float2(1.0, 0.0);
        float _e47 = candidate_distance_squared;
        nearest_distance_squared = _e47;
    }
    candidate_delta = point_3 - metal::float2(metal::clamp(point_3.x, horizontal_extents.w, metal::max(horizontal_extents.w, size_2.x - horizontal_extents.z)), size_2.y);
    metal::float2 _e59 = candidate_delta;
    metal::float2 _e60 = candidate_delta;
    candidate_distance_squared = metal::dot(_e59, _e60);
    float _e62 = candidate_distance_squared;
    float _e63 = nearest_distance_squared;
    if (_e62 < _e63) {
        metal::float2 _e65 = candidate_delta;
        nearest_delta = _e65;
        nearest_normal = metal::float2(0.0, 1.0);
        float _e69 = candidate_distance_squared;
        nearest_distance_squared = _e69;
    }
    candidate_delta = point_3 - metal::float2(0.0, metal::clamp(point_3.y, vertical_extents.x, metal::max(vertical_extents.x, size_2.y - vertical_extents.w)));
    metal::float2 _e81 = candidate_delta;
    metal::float2 _e82 = candidate_delta;
    candidate_distance_squared = metal::dot(_e81, _e82);
    float _e84 = candidate_distance_squared;
    float _e85 = nearest_distance_squared;
    if (_e84 < _e85) {
        metal::float2 _e87 = candidate_delta;
        nearest_delta = _e87;
        nearest_normal = metal::float2(-1.0, 0.0);
        float _e91 = candidate_distance_squared;
        nearest_distance_squared = _e91;
    }
    metal::float2 _e92 = nearest_delta;
    metal::float2 _e93 = nearest_normal;
    float _e94 = nearest_distance_squared;
    float _e96 = figma_signed_distance(_e92, _e93, metal::sqrt(_e94));
    metal::float2 _e97 = nearest_normal;
    return SignedDistanceSample {_e96, {}, _e97, 0.0, FIGMA_SEGMENT_STRAIGHT};
}

bool figma_is_corner_candidate(
    metal::float2 point_4,
    metal::float2 size_3,
    float horizontal_extent,
    float vertical_extent,
    uint corner
) {
    bool local_21 = {};
    bool local_22 = {};
    bool local_23 = {};
    bool local_24 = {};
    bool local_25 = {};
    if (!((horizontal_extent <= FIGMA_EPSILON))) {
        local_21 = vertical_extent <= FIGMA_EPSILON;
    } else {
        local_21 = true;
    }
    bool _e13 = local_21;
    if (_e13) {
        return false;
    }
    switch(corner) {
        case 0u: {
            if (point_4.x <= horizontal_extent) {
                local_22 = point_4.y <= vertical_extent;
            } else {
                local_22 = false;
            }
            bool _e22 = local_22;
            return _e22;
        }
        case 1u: {
            if ((size_3.x - point_4.x) <= horizontal_extent) {
                local_23 = point_4.y <= vertical_extent;
            } else {
                local_23 = false;
            }
            bool _e32 = local_23;
            return _e32;
        }
        case 2u: {
            if ((size_3.x - point_4.x) <= horizontal_extent) {
                local_24 = (size_3.y - point_4.y) <= vertical_extent;
            } else {
                local_24 = false;
            }
            bool _e44 = local_24;
            return _e44;
        }
        default: {
            if (point_4.x <= horizontal_extent) {
                local_25 = (size_3.y - point_4.y) <= vertical_extent;
            } else {
                local_25 = false;
            }
            bool _e54 = local_25;
            return _e54;
        }
    }
}

metal::float2 figma_corner_to_point(
    metal::float2 point_5,
    metal::float2 size_4,
    uint corner_1
) {
    switch(corner_1) {
        case 0u: {
            return -(point_5);
        }
        case 1u: {
            return metal::float2(point_5.x - size_4.x, -(point_5.y));
        }
        case 2u: {
            return point_5 - size_4;
        }
        default: {
            return metal::float2(-(point_5.x), point_5.y - size_4.y);
        }
    }
}

float reach_aware_normalized_superellipse_signed_distance(
    metal::float2 point_6,
    Bounds bounds_8,
    Corners corner_radii_5,
    float corner_smoothing_5,
    metal::float4 reaches_1,
    float power_2
) {
    float distance_2 = {};
    uint corner_2 = 0u;
    metal::float2 local_point = point_6 - bounds_8.origin;
    metal::float4 _e8 = corner_values(corner_radii_5);
    SignedDistanceSample _e10 = figma_nearest_straight_sample(local_point, bounds_8.size, reaches_1, reaches_1);
    distance_2 = _e10.distance;
    uint2 loop_bound = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        uint _e15 = corner_2;
        if (_e15 < 4u) {
        } else {
            break;
        }
        {
            uint _e19 = corner_2;
            uint _e21 = corner_2;
            uint _e23 = corner_2;
            bool _e24 = figma_is_corner_candidate(local_point, bounds_8.size, reaches_1[_e19], reaches_1[_e21], _e23);
            if (_e24) {
                uint _e26 = corner_2;
                metal::float2 _e27 = figma_corner_to_point(local_point, bounds_8.size, _e26);
                uint _e28 = corner_2;
                float _e30 = normalized_superellipse_signed_distance_from_corner(_e27, _e8[_e28], corner_smoothing_5, power_2);
                float _e32 = distance_2;
                if (metal::abs(_e30) <= metal::abs(_e32)) {
                    distance_2 = _e30;
                }
            }
            uint _e35 = corner_2;
            corner_2 = _e35 + 1u;
        }
    }
    float _e38 = distance_2;
    return _e38;
}

float normalized_superellipse_signed_distance(
    metal::float2 point_7,
    Bounds bounds_9,
    Corners corner_radii_6,
    float corner_smoothing_6,
    metal::float4 reaches_2,
    float power_3
) {
    bool _e7 = can_use_compact_corner_selection(bounds_9.size, reaches_2);
    if (_e7) {
        float _e8 = compact_normalized_superellipse_signed_distance(point_7, bounds_9, corner_radii_6, corner_smoothing_6, power_3);
        return _e8;
    }
    float _e9 = reach_aware_normalized_superellipse_signed_distance(point_7, bounds_9, corner_radii_6, corner_smoothing_6, reaches_2, power_3);
    return _e9;
}

metal::float2 figma_split_side(
    float length,
    float first_radius,
    float second_radius
) {
    float total_radius = first_radius + second_radius;
    if (total_radius == 0.0) {
        return metal::float2(0.0, 0.0);
    }
    float first_budget = (length * first_radius) / total_radius;
    return metal::float2(first_budget, length - first_budget);
}

FigmaCornerLayout figma_corner_layout(
    metal::float2 size_5,
    Corners corner_radii_7
) {
    type_12 radii_2 = {};
    type_12 budgets = type_12 {{-1.0, -1.0, -1.0, -1.0}};
    type_13 order = type_13 {{0u, 1u, 3u, 2u}};
    uint sort_pass = 0u;
    uint index = {};
    uint rank = 0u;
    uint horizontal_neighbor = {};
    uint vertical_neighbor = {};
    float horizontal_budget = {};
    bool local_26 = {};
    float vertical_budget = {};
    bool local_27 = {};
    radii_2 = type_12 {{metal::max(corner_radii_7.top_left, 0.0), metal::max(corner_radii_7.top_right, 0.0), metal::max(corner_radii_7.bottom_right, 0.0), metal::max(corner_radii_7.bottom_left, 0.0)}};
    uint2 loop_bound_1 = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        uint _e30 = sort_pass;
        if (_e30 < 3u) {
        } else {
            break;
        }
        {
            index = 0u;
            uint2 loop_bound_2 = uint2(4294967295u);
            while(true) {
                if (metal::all(loop_bound_2 == uint2(0u))) { break; }
                loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
                uint _e35 = index;
                uint _e37 = sort_pass;
                if (_e35 < (3u - _e37)) {
                } else {
                    break;
                }
                {
                    uint _e40 = index;
                    uint _e43 = order.inner[static_cast<uint>(_e40)];
                    float _e46 = radii_2.inner[static_cast<uint>(_e43)];
                    uint _e47 = index;
                    uint _e52 = order.inner[static_cast<uint>(_e47 + 1u)];
                    float _e55 = radii_2.inner[static_cast<uint>(_e52)];
                    if (_e46 < _e55) {
                        uint _e57 = index;
                        uint swap = order.inner[static_cast<uint>(_e57)];
                        uint _e61 = index;
                        uint _e64 = index;
                        uint _e69 = order.inner[static_cast<uint>(_e64 + 1u)];
                        order.inner[static_cast<uint>(_e61)] = _e69;
                        uint _e70 = index;
                        order.inner[static_cast<uint>(_e70 + 1u)] = swap;
                    }
                    uint _e75 = index;
                    index = _e75 + 1u;
                }
            }
            uint _e78 = sort_pass;
            sort_pass = _e78 + 1u;
        }
    }
    uint2 loop_bound_3 = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
        uint _e83 = rank;
        if (_e83 < 4u) {
        } else {
            break;
        }
        {
            uint _e86 = rank;
            uint corner_5 = order.inner[static_cast<uint>(_e86)];
            float radius = radii_2.inner[static_cast<uint>(corner_5)];
            horizontal_neighbor = 0u;
            vertical_neighbor = 0u;
            switch(corner_5) {
                case 0u: {
                    horizontal_neighbor = 1u;
                    vertical_neighbor = 3u;
                    break;
                }
                case 1u: {
                    horizontal_neighbor = 0u;
                    vertical_neighbor = 2u;
                    break;
                }
                case 2u: {
                    horizontal_neighbor = 3u;
                    vertical_neighbor = 1u;
                    break;
                }
                default: {
                    horizontal_neighbor = 2u;
                    vertical_neighbor = 0u;
                    break;
                }
            }
            uint _e105 = horizontal_neighbor;
            float horizontal_radius = radii_2.inner[static_cast<uint>(_e105)];
            horizontal_budget = 0.0;
            if (!((radius != 0.0))) {
                local_26 = horizontal_radius != 0.0;
            } else {
                local_26 = true;
            }
            bool _e119 = local_26;
            if (_e119) {
                uint _e120 = horizontal_neighbor;
                float _e123 = budgets.inner[static_cast<uint>(_e120)];
                if (_e123 >= 0.0) {
                    uint _e127 = horizontal_neighbor;
                    float _e130 = budgets.inner[static_cast<uint>(_e127)];
                    horizontal_budget = size_5.x - _e130;
                } else {
                    horizontal_budget = (size_5.x * radius) / (radius + horizontal_radius);
                }
            }
            uint _e136 = vertical_neighbor;
            float vertical_radius = radii_2.inner[static_cast<uint>(_e136)];
            vertical_budget = 0.0;
            if (!((radius != 0.0))) {
                local_27 = vertical_radius != 0.0;
            } else {
                local_27 = true;
            }
            bool _e150 = local_27;
            if (_e150) {
                uint _e151 = vertical_neighbor;
                float _e154 = budgets.inner[static_cast<uint>(_e151)];
                if (_e154 >= 0.0) {
                    uint _e158 = vertical_neighbor;
                    float _e161 = budgets.inner[static_cast<uint>(_e158)];
                    vertical_budget = size_5.y - _e161;
                } else {
                    vertical_budget = (size_5.y * radius) / (radius + vertical_radius);
                }
            }
            float _e167 = horizontal_budget;
            float _e168 = vertical_budget;
            float budget = metal::max(0.0, metal::min(_e167, _e168));
            budgets.inner[static_cast<uint>(corner_5)] = budget;
            radii_2.inner[static_cast<uint>(corner_5)] = metal::min(radius, budget);
            uint _e177 = rank;
            rank = _e177 + 1u;
        }
    }
    float _e182 = radii_2.inner[0];
    float _e184 = radii_2.inner[1];
    metal::float2 _e185 = figma_split_side(size_5.x, _e182, _e184);
    float _e188 = radii_2.inner[3];
    float _e190 = radii_2.inner[2];
    metal::float2 _e191 = figma_split_side(size_5.x, _e188, _e190);
    float _e194 = radii_2.inner[0];
    float _e196 = radii_2.inner[3];
    metal::float2 _e197 = figma_split_side(size_5.y, _e194, _e196);
    float _e200 = radii_2.inner[1];
    float _e202 = radii_2.inner[2];
    metal::float2 _e203 = figma_split_side(size_5.y, _e200, _e202);
    return FigmaCornerLayout {metal::float4(_e185.x, _e185.y, _e191.y, _e191.x), metal::float4(_e197.x, _e203.x, _e203.y, _e197.y)};
}

metal::float4 figma_smoothing_factors(
    float corner_smoothing_7
) {
    float smoothing_2 = metal::clamp(corner_smoothing_7, 0.0, 1.0);
    float arc_sweep = 1.5707964 * (1.0 - smoothing_2);
    float beta = 0.7853982 * smoothing_2;
    float join_handle_factor = metal::tan(0.5 * beta);
    return metal::float4(smoothing_2, metal::sin(0.5 * arc_sweep) * 1.4142135, join_handle_factor * metal::cos(beta), join_handle_factor * metal::sin(beta));
}

FigmaCornerParams figma_corner_params(
    float corner_radius_3,
    float horizontal_reach,
    float vertical_reach,
    metal::float4 smoothing_factors
) {
    float safe_horizontal_reach = metal::max(horizontal_reach, 0.0);
    float safe_vertical_reach = metal::max(vertical_reach, 0.0);
    float radius_1 = metal::min(metal::max(corner_radius_3, 0.0), metal::min(safe_horizontal_reach, safe_vertical_reach));
    float smoothing_3 = (radius_1 > FIGMA_EPSILON) ? smoothing_factors.x : 0.0;
    float desired_reach = radius_1 * (1.0 + smoothing_3);
    float arc_sweep_1 = 1.5707964 * (1.0 - smoothing_3);
    float arc_delta = smoothing_factors.y * radius_1;
    float c_4 = smoothing_factors.z * radius_1;
    float d_4 = smoothing_factors.w * radius_1;
    float core_length = (arc_delta + c_4) + d_4;
    float ideal_b = metal::max((desired_reach - core_length) / 3.0, 0.0);
    float horizontal_available = metal::max(safe_horizontal_reach - core_length, 0.0);
    float vertical_available = metal::max(safe_vertical_reach - core_length, 0.0);
    float shared_b = metal::min(ideal_b, metal::min(horizontal_available * 0.8333333, vertical_available * 0.8333333));
    return FigmaCornerParams {radius_1, smoothing_3, arc_sweep_1, c_4, d_4, FigmaAxisParams {horizontal_available - shared_b, shared_b, safe_horizontal_reach}, FigmaAxisParams {vertical_available - shared_b, shared_b, safe_vertical_reach}};
}

FigmaCornerExtents figma_corner_extents(
    Corners corner_radii_8,
    metal::float4 horizontal_budgets,
    metal::float4 vertical_budgets,
    float corner_smoothing_8
) {
    metal::float4 zero = metal::float4(0.0, 0.0, 0.0, 0.0);
    metal::float4 horizontal = metal::max(horizontal_budgets, zero);
    metal::float4 vertical = metal::max(vertical_budgets, zero);
    metal::float4 _e11 = corner_values(corner_radii_8);
    metal::float4 radii_5 = metal::min(metal::max(_e11, zero), metal::min(horizontal, vertical));
    metal::float4 reaches_3 = radii_5 * (1.0 + metal::clamp(corner_smoothing_8, 0.0, 1.0));
    return FigmaCornerExtents {metal::min(reaches_3, horizontal), metal::min(reaches_3, vertical)};
}

PreparedCorners prepare_corners(
    metal::float2 size_6,
    Corners radii_3,
    float smoothing,
    bool allow_superellipse
) {
    PreparedCorners prepared_3 = {};
    bool local_28 = {};
    metal::float4 _e4 = corner_values(radii_3);
    prepared_3 = PreparedCorners {_e4, _e4, metal::float4(0.0, 1.0, 0.0, 0.0), 0.0};
    if (smoothing <= 0.0) {
        PreparedCorners _e15 = prepared_3;
        return _e15;
    }
    if (allow_superellipse) {
        bool _e18 = can_use_normalized_superellipse(size_6, radii_3, smoothing);
        local_28 = _e18;
    } else {
        local_28 = false;
    }
    bool _e20 = local_28;
    if (_e20) {
        metal::float4 _e21 = normalized_superellipse_reaches(radii_3, smoothing);
        prepared_3.horizontal_reaches = _e21;
        prepared_3.vertical_reaches = _e21;
        float _e25 = normalized_superellipse_power(smoothing);
        prepared_3.superellipse_power = _e25;
        PreparedCorners _e26 = prepared_3;
        return _e26;
    }
    FigmaCornerLayout _e27 = figma_corner_layout(size_6, radii_3);
    FigmaCornerExtents _e30 = figma_corner_extents(radii_3, _e27.horizontal_budgets, _e27.vertical_budgets, smoothing);
    prepared_3.horizontal_reaches = _e30.horizontal;
    prepared_3.vertical_reaches = _e30.vertical;
    metal::float4 _e36 = figma_smoothing_factors(smoothing);
    prepared_3.smoothing_factors = _e36;
    PreparedCorners _e37 = prepared_3;
    return _e37;
}

metal::float2 figma_cubic_point(
    FigmaAxisParams axis,
    float c,
    float d,
    float t
) {
    float x1_ = 3.0 * axis.a;
    float x2_ = 3.0 * (axis.b - axis.a);
    float x3_ = (axis.a - (2.0 * axis.b)) + c;
    float t2_ = t * t;
    return metal::float2(t * (x1_ + (t * (x2_ + (t * x3_)))), (d * t2_) * t);
}

metal::float2 figma_cubic_derivative(
    FigmaAxisParams axis_1,
    float c_1,
    float d_1,
    float t_1
) {
    float x1_1 = 3.0 * axis_1.a;
    float x2_1 = 3.0 * (axis_1.b - axis_1.a);
    float x3_1 = (axis_1.a - (2.0 * axis_1.b)) + c_1;
    float t2_1 = t_1 * t_1;
    return metal::float2((x1_1 + ((2.0 * x2_1) * t_1)) + ((3.0 * x3_1) * t2_1), (3.0 * d_1) * t2_1);
}

metal::float2 figma_cubic_second_derivative(
    FigmaAxisParams axis_2,
    float c_2,
    float d_2,
    float t_2
) {
    float x2_2 = 3.0 * (axis_2.b - axis_2.a);
    float x3_2 = (axis_2.a - (2.0 * axis_2.b)) + c_2;
    return metal::float2((2.0 * x2_2) + ((6.0 * x3_2) * t_2), (6.0 * d_2) * t_2);
}

CubicClosestPoint closest_figma_cubic(
    metal::float2 point_8,
    FigmaAxisParams axis_3,
    float c_3,
    float d_3
) {
    float t_3 = {};
    uint iteration = 0u;
    float closest_t = {};
    metal::float2 closest_point = {};
    float closest_distance = {};
    float y_seed = metal::pow(metal::clamp(point_8.y / metal::max(d_3, 0.00001), 0.0, 1.0), 0.33333334);
    metal::float2 _e14 = figma_cubic_point(axis_3, c_3, d_3, 1.0);
    float chord_seed = metal::clamp(metal::dot(point_8, _e14) / metal::max(metal::dot(_e14, _e14), FIGMA_EPSILON), 0.0, 1.0);
    metal::float2 _e23 = figma_cubic_point(axis_3, c_3, d_3, y_seed);
    metal::float2 y_delta = _e23 - point_8;
    metal::float2 _e25 = figma_cubic_point(axis_3, c_3, d_3, chord_seed);
    metal::float2 chord_delta = _e25 - point_8;
    t_3 = (metal::dot(chord_delta, chord_delta) < metal::dot(y_delta, y_delta)) ? chord_seed : y_seed;
    uint2 loop_bound_4 = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound_4 == uint2(0u))) { break; }
        loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
        uint _e34 = iteration;
        if (_e34 < 4u) {
        } else {
            break;
        }
        {
            float _e37 = t_3;
            metal::float2 _e38 = figma_cubic_point(axis_3, c_3, d_3, _e37);
            float _e39 = t_3;
            metal::float2 _e40 = figma_cubic_derivative(axis_3, c_3, d_3, _e39);
            float _e41 = t_3;
            metal::float2 _e42 = figma_cubic_second_derivative(axis_3, c_3, d_3, _e41);
            metal::float2 delta_1 = _e38 - point_8;
            float denominator = metal::dot(_e40, _e40) + metal::dot(delta_1, _e42);
            if (metal::abs(denominator) > FIGMA_EPSILON) {
                float _e50 = t_3;
                t_3 = metal::clamp(_e50 - (metal::dot(delta_1, _e40) / denominator), 0.0, 1.0);
            }
            uint _e57 = iteration;
            iteration = _e57 + 1u;
        }
    }
    float _e60 = t_3;
    closest_t = _e60;
    float _e62 = t_3;
    metal::float2 _e63 = figma_cubic_point(axis_3, c_3, d_3, _e62);
    closest_point = _e63;
    metal::float2 _e65 = closest_point;
    closest_distance = metal::length(point_8 - _e65);
    metal::float2 start = metal::float2(0.0, 0.0);
    float start_distance = metal::length(point_8 - start);
    float _e74 = closest_distance;
    if (start_distance < _e74) {
        closest_t = 0.0;
        closest_point = start;
        closest_distance = start_distance;
    }
    float end_distance = metal::length(point_8 - _e14);
    float _e79 = closest_distance;
    if (end_distance < _e79) {
        closest_t = 1.0;
        closest_point = _e14;
        closest_distance = end_distance;
    }
    metal::float2 _e82 = closest_point;
    float _e83 = closest_t;
    metal::float2 _e84 = figma_cubic_derivative(axis_3, c_3, d_3, _e83);
    float _e85 = closest_distance;
    float _e86 = closest_t;
    return CubicClosestPoint {_e82, _e84, _e85, _e86};
}

float figma_cross_2d(
    metal::float2 a,
    metal::float2 b
) {
    return (a.x * b.y) - (a.y * b.x);
}

metal::float2 figma_unfold_normal(
    metal::float2 normal_1,
    bool mirrored
) {
    return mirrored ? metal::float2(-(normal_1.y), normal_1.x) : metal::float2(normal_1.x, -(normal_1.y));
}

SignedDistanceSample figma_corner_signed_distance(
    metal::float2 corner_to_point_1,
    FigmaCornerParams params
) {
    bool local_29 = {};
    bool local_30 = {};
    SignedDistanceSample sample_2 = {};
    metal::float2 z = corner_to_point_1 + metal::float2(params.horizontal.p, params.vertical.p);
    if (!((params.radius <= FIGMA_EPSILON))) {
        local_29 = z.x <= 0.0;
    } else {
        local_29 = true;
    }
    bool _e18 = local_29;
    if (!(_e18)) {
        local_30 = z.y <= 0.0;
    } else {
        local_30 = true;
    }
    bool _e26 = local_30;
    if (_e26) {
        return SignedDistanceSample {metal::max(corner_to_point_1.x, corner_to_point_1.y), {}, (corner_to_point_1.x > corner_to_point_1.y) ? metal::float2(1.0, 0.0) : metal::float2(0.0, 1.0), 0.0, FIGMA_SEGMENT_STRAIGHT};
    }
    metal::float2 horizontal_point = metal::float2(z.x, params.vertical.p - z.y);
    metal::float2 vertical_point = metal::float2(z.y, params.horizontal.p - z.x);
    metal::float2 circle_center = metal::float2(params.horizontal.p - params.radius, params.radius);
    metal::float2 join = metal::float2((params.horizontal.a + params.horizontal.b) + params.c, params.d);
    metal::float2 start_direction = (join - circle_center) / metal::float2(params.radius);
    metal::float2 to_point = horizontal_point - circle_center;
    float to_point_length = metal::length(to_point);
    metal::float2 point_direction = (to_point_length > FIGMA_EPSILON) ? (to_point / metal::float2(metal::max(to_point_length, FIGMA_EPSILON))) : start_direction;
    float _e83 = figma_cross_2d(start_direction, point_direction);
    float arc_angle = metal::clamp(metal::atan2(_e83, metal::dot(start_direction, point_direction)), 0.0, params.arc_sweep);
    float arc_sine = metal::sin(arc_angle);
    float arc_cosine = metal::cos(arc_angle);
    metal::float2 arc_normal = metal::float2((arc_cosine * start_direction.x) - (arc_sine * start_direction.y), (arc_sine * start_direction.x) + (arc_cosine * start_direction.y));
    metal::float2 arc_point = circle_center + (params.radius * arc_normal);
    metal::float2 arc_delta_1 = horizontal_point - arc_point;
    float arc_t = (params.arc_sweep > FIGMA_EPSILON) ? (arc_angle / metal::max(params.arc_sweep, FIGMA_EPSILON)) : 0.0;
    float _e116 = figma_signed_distance(arc_delta_1, arc_normal, metal::length(arc_delta_1));
    metal::float2 _e118 = figma_unfold_normal(arc_normal, false);
    sample_2 = SignedDistanceSample {_e116, {}, _e118, arc_t, FIGMA_SEGMENT_ARC};
    if (params.smoothing > FIGMA_EPSILON) {
        metal::float2 horizontal_bounds_delta = metal::max(metal::float2(0.0, 0.0), metal::max(-(horizontal_point), horizontal_point - join));
        float _e134 = sample_2.distance;
        float _e136 = sample_2.distance;
        if (metal::dot(horizontal_bounds_delta, horizontal_bounds_delta) <= (((_e134 * _e136) * 1.000001) + FIGMA_EPSILON)) {
            CubicClosestPoint _e146 = closest_figma_cubic(horizontal_point, params.horizontal, params.c, params.d);
            metal::float2 normal_3 = metal::normalize(metal::float2(_e146.tangent.y, -(_e146.tangent.x)));
            float _e157 = figma_signed_distance(horizontal_point - _e146.point, normal_3, _e146.distance);
            float _e160 = sample_2.distance;
            if (metal::abs(_e157) <= metal::abs(_e160)) {
                sample_2.distance = _e157;
                metal::float2 _e166 = figma_unfold_normal(normal_3, false);
                sample_2.normal = _e166;
                sample_2.path_t = _e146.path_t;
                sample_2.segment = FIGMA_SEGMENT_FIRST_CUBIC;
            }
        }
        metal::float2 vertical_join = metal::float2((params.vertical.a + params.vertical.b) + params.c, params.d);
        metal::float2 vertical_bounds_delta = metal::max(metal::float2(0.0, 0.0), metal::max(-(vertical_point), vertical_point - vertical_join));
        float _e189 = sample_2.distance;
        float _e191 = sample_2.distance;
        if (metal::dot(vertical_bounds_delta, vertical_bounds_delta) <= (((_e189 * _e191) * 1.000001) + FIGMA_EPSILON)) {
            CubicClosestPoint _e201 = closest_figma_cubic(vertical_point, params.vertical, params.c, params.d);
            metal::float2 normal_4 = metal::normalize(metal::float2(_e201.tangent.y, -(_e201.tangent.x)));
            float _e212 = figma_signed_distance(vertical_point - _e201.point, normal_4, _e201.distance);
            float _e215 = sample_2.distance;
            if (metal::abs(_e212) <= metal::abs(_e215)) {
                sample_2.distance = _e212;
                metal::float2 _e221 = figma_unfold_normal(normal_4, true);
                sample_2.normal = _e221;
                sample_2.path_t = _e201.path_t;
                sample_2.segment = FIGMA_SEGMENT_SECOND_CUBIC;
            }
        }
    }
    SignedDistanceSample _e226 = sample_2;
    return _e226;
}

float figma_cubic_length(
    FigmaCornerParams params_1,
    FigmaAxisParams axis_4,
    float end_t
) {
    bool local_31 = {};
    float center = metal::clamp(end_t, 0.0, 1.0) / 2.0;
    if (!((center <= 0.0))) {
        local_31 = params_1.smoothing <= 0.0;
    } else {
        local_31 = true;
    }
    bool _e17 = local_31;
    if (_e17) {
        return 0.0;
    }
    float offset1_ = center * 0.5384693;
    float offset2_ = center * 0.90617985;
    metal::float2 _e25 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, center);
    float speed0_ = metal::length(_e25);
    metal::float2 _e30 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, center - offset1_);
    metal::float2 _e35 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, center + offset1_);
    float speed1_ = metal::length(_e30) + metal::length(_e35);
    metal::float2 _e41 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, center - offset2_);
    metal::float2 _e46 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, center + offset2_);
    float speed2_ = metal::length(_e41) + metal::length(_e46);
    return center * (((0.5688889 * speed0_) + (0.47862867 * speed1_)) + (0.23692688 * speed2_));
}

float figma_corner_length(
    FigmaCornerParams params_2
) {
    float _e3 = figma_cubic_length(params_2, params_2.horizontal, 1.0);
    float _e10 = figma_cubic_length(params_2, params_2.vertical, 1.0);
    return (_e3 + (params_2.radius * params_2.arc_sweep)) + _e10;
}

float figma_corner_progress(
    FigmaCornerParams params_3,
    SignedDistanceSample sample_3,
    float total_length
) {
    if (sample_3.segment == FIGMA_SEGMENT_FIRST_CUBIC) {
        float _e8 = figma_cubic_length(params_3, params_3.horizontal, sample_3.path_t);
        return _e8;
    }
    if (sample_3.segment == FIGMA_SEGMENT_ARC) {
        float _e14 = figma_cubic_length(params_3, params_3.horizontal, 1.0);
        return _e14 + ((params_3.radius * params_3.arc_sweep) * sample_3.path_t);
    }
    if (sample_3.segment == FIGMA_SEGMENT_SECOND_CUBIC) {
        float _e26 = figma_cubic_length(params_3, params_3.vertical, sample_3.path_t);
        return total_length - _e26;
    }
    return 0.0;
}

bool figma_has_corner_candidate(
    metal::float2 point_9,
    metal::float2 size_7,
    metal::float4 horizontal_extents_1,
    metal::float4 vertical_extents_1
) {
    bool local_32 = {};
    bool local_33 = {};
    bool local_34 = {};
    bool _e7 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.x, vertical_extents_1.x, 0u);
    if (!(_e7)) {
        bool _e14 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.y, vertical_extents_1.y, 1u);
        local_32 = _e14;
    } else {
        local_32 = true;
    }
    bool _e16 = local_32;
    if (!(_e16)) {
        bool _e23 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.z, vertical_extents_1.z, 2u);
        local_33 = _e23;
    } else {
        local_33 = true;
    }
    bool _e25 = local_33;
    if (!(_e25)) {
        bool _e32 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.w, vertical_extents_1.w, 3u);
        local_34 = _e32;
    } else {
        local_34 = true;
    }
    bool _e34 = local_34;
    return _e34;
}

metal::float2 figma_orient_corner_normal(
    metal::float2 normal_2,
    uint corner_3
) {
    switch(corner_3) {
        case 0u: {
            return -(normal_2);
        }
        case 1u: {
            return metal::float2(normal_2.x, -(normal_2.y));
        }
        case 2u: {
            return normal_2;
        }
        default: {
            return metal::float2(-(normal_2.x), normal_2.y);
        }
    }
}

FigmaRectangleSample figma_smooth_rectangle_sample(
    metal::float2 point_10,
    Bounds bounds_10,
    Corners corner_radii_9,
    metal::float4 horizontal_reaches,
    metal::float4 vertical_reaches,
    metal::float4 smoothing_factors_1
) {
    FigmaRectangleSample result = {};
    uint corner_4 = 0u;
    SignedDistanceSample candidate = {};
    metal::float2 local_point_1 = point_10 - bounds_10.origin;
    metal::float4 _e8 = corner_values(corner_radii_9);
    SignedDistanceSample _e10 = figma_nearest_straight_sample(local_point_1, bounds_10.size, horizontal_reaches, vertical_reaches);
    result = FigmaRectangleSample {_e10, FIGMA_NO_CORNER};
    bool _e15 = figma_has_corner_candidate(local_point_1, bounds_10.size, horizontal_reaches, vertical_reaches);
    if (!(_e15)) {
        FigmaRectangleSample _e17 = result;
        return _e17;
    }
    uint2 loop_bound_5 = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound_5 == uint2(0u))) { break; }
        loop_bound_5 -= uint2(loop_bound_5.y == 0u, 1u);
        uint _e20 = corner_4;
        if (_e20 < 4u) {
        } else {
            break;
        }
        {
            uint _e24 = corner_4;
            uint _e26 = corner_4;
            uint _e28 = corner_4;
            bool _e29 = figma_is_corner_candidate(local_point_1, bounds_10.size, horizontal_reaches[_e24], vertical_reaches[_e26], _e28);
            if (_e29) {
                uint _e30 = corner_4;
                uint _e32 = corner_4;
                uint _e34 = corner_4;
                FigmaCornerParams _e36 = figma_corner_params(_e8[_e30], horizontal_reaches[_e32], vertical_reaches[_e34], smoothing_factors_1);
                if (_e36.radius > FIGMA_EPSILON) {
                    uint _e41 = corner_4;
                    metal::float2 _e42 = figma_corner_to_point(local_point_1, bounds_10.size, _e41);
                    SignedDistanceSample _e43 = figma_corner_signed_distance(_e42, _e36);
                    candidate = _e43;
                    metal::float2 _e47 = candidate.normal;
                    uint _e48 = corner_4;
                    metal::float2 _e49 = figma_orient_corner_normal(_e47, _e48);
                    candidate.normal = _e49;
                    float _e51 = candidate.distance;
                    float _e55 = result.signed_distance.distance;
                    if (metal::abs(_e51) <= metal::abs(_e55)) {
                        SignedDistanceSample _e59 = candidate;
                        result.signed_distance = _e59;
                        uint _e61 = corner_4;
                        result.corner = _e61;
                    }
                }
            }
            uint _e62 = corner_4;
            corner_4 = _e62 + 1u;
        }
    }
    FigmaRectangleSample _e65 = result;
    return _e65;
}

float prepared_corner_signed_distance(
    metal::float2 point_11,
    Bounds bounds_11,
    Corners corner_radii_10,
    float corner_smoothing_9,
    PreparedCorners prepared_4
) {
    if (prepared_4.superellipse_power > 0.0) {
        float _e10 = normalized_superellipse_signed_distance(point_11, bounds_11, corner_radii_10, corner_smoothing_9, prepared_4.horizontal_reaches, prepared_4.superellipse_power);
        return _e10;
    }
    if (prepared_4.smoothing_factors.x <= 0.0) {
        float _e15 = rounded_rectangle_signed_distance(point_11, bounds_11, corner_radii_10);
        return _e15;
    }
    FigmaRectangleSample _e19 = figma_smooth_rectangle_sample(point_11, bounds_11, corner_radii_10, prepared_4.horizontal_reaches, prepared_4.vertical_reaches, prepared_4.smoothing_factors);
    return _e19.signed_distance.distance;
}

Bounds shadow_geometry(
    Shadow shadow
) {
    bool _e2 = is_enabled(shadow.inset);
    if (_e2) {
        return shadow.element_bounds;
    }
    float margin = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow.blur_radius;
    return Bounds {shadow.bounds.origin - metal::float2(margin, margin), shadow.bounds.size + metal::float2(2.0 * margin, 2.0 * margin)};
}

Paint shadow_paint(
    Shadow shadow_1
) {
    Bounds bounds_12 = {};
    bounds_12 = shadow_1.bounds;
    bool _e4 = is_enabled(shadow_1.inset);
    if (_e4) {
        bounds_12 = shadow_1.element_bounds;
    }
    Bounds _e7 = bounds_12;
    Paint _e8 = Paint_new(shadow_1.color, _e7);
    return _e8;
}

float blurred_shadow_coverage(
    Shadow shadow_2,
    metal::float2 position_12
) {
    float coverage_1 = 0.0;
    int sample_index = 0;
    float sample_position = {};
    metal::float2 _e3 = Bounds_1half_size(shadow_2.bounds);
    metal::float2 _e5 = Bounds_center(shadow_2.bounds);
    metal::float2 center_to_point_3 = position_12 - _e5;
    float _e8 = pick_corner_radius(center_to_point_3, shadow_2.corner_radii);
    float vertical_minimum = center_to_point_3.y - _e3.y;
    float vertical_maximum = center_to_point_3.y + _e3.y;
    float integration_radius = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow_2.blur_radius;
    float integration_start = metal::clamp(-(integration_radius), vertical_minimum, vertical_maximum);
    float integration_end = metal::clamp(integration_radius, vertical_minimum, vertical_maximum);
    float sample_step = (integration_end - integration_start) / 4.0;
    sample_position = integration_start + (sample_step * 0.5);
    uint2 loop_bound_6 = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound_6 == uint2(0u))) { break; }
        loop_bound_6 -= uint2(loop_bound_6.y == 0u, 1u);
        int _e32 = sample_index;
        if (_e32 < SHADOW_INTEGRATION_SAMPLE_COUNT) {
        } else {
            break;
        }
        {
            float _e35 = coverage_1;
            float _e38 = sample_position;
            float _e41 = integrated_rounded_rectangle_coverage(center_to_point_3.x, center_to_point_3.y - _e38, shadow_2.blur_radius, _e8, _e3);
            float _e42 = sample_position;
            float _e44 = gaussian(_e42, shadow_2.blur_radius);
            coverage_1 = _e35 + ((_e41 * _e44) * sample_step);
            float _e48 = sample_position;
            sample_position = _e48 + sample_step;
            int _e50 = sample_index;
            sample_index = as_type<int>(as_type<uint>(_e50) + as_type<uint>(1));
        }
    }
    float _e53 = coverage_1;
    return _e53;
}

float shadow_coverage(
    Shadow shadow_3,
    metal::float2 position_13
) {
    float coverage_2 = 0.0;
    if (shadow_3.blur_radius == 0.0) {
        float _e9 = rounded_rectangle_signed_distance(position_13, shadow_3.bounds, shadow_3.corner_radii);
        float _e10 = antialiased_coverage(_e9);
        coverage_2 = _e10;
    } else {
        float _e11 = blurred_shadow_coverage(shadow_3, position_13);
        coverage_2 = _e11;
    }
    bool _e13 = is_enabled(shadow_3.inset);
    if (_e13) {
        float _e14 = coverage_2;
        float _e19 = rounded_rectangle_signed_distance(position_13, shadow_3.element_bounds, shadow_3.element_corner_radii);
        float _e20 = antialiased_coverage(_e19);
        coverage_2 = (1.0 - _e14) * _e20;
    }
    float _e22 = coverage_2;
    return _e22;
}

float smoothed_shadow_coverage(
    Shadow shadow_4,
    metal::float2 position_14,
    PreparedCorners prepared_5,
    PreparedCorners prepared_element
) {
    float coverage_3 = 0.0;
    bool local_35 = {};
    if (shadow_4.blur_radius == 0.0) {
        float _e12 = prepared_corner_signed_distance(position_14, shadow_4.bounds, shadow_4.corner_radii, shadow_4.corner_smoothing, prepared_5);
        float _e13 = antialiased_coverage(_e12);
        coverage_3 = _e13;
    } else {
        bool _e15 = Corners_1is_zero(shadow_4.corner_radii);
        if (!(_e15)) {
            float blur_limit = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow_4.blur_radius;
            metal::float2 _e21 = Bounds_1half_size(shadow_4.bounds);
            metal::float2 _e23 = Bounds_center(shadow_4.bounds);
            metal::float2 center_to_point_4 = position_14 - _e23;
            metal::float2 box_delta = metal::abs(center_to_point_4) - _e21;
            metal::float2 outside_delta = metal::max(box_delta, metal::float2(0.0, 0.0));
            if (metal::dot(outside_delta, outside_delta) <= (blur_limit * blur_limit)) {
                metal::float2 local_point_2 = position_14 - shadow_4.bounds.origin;
                float edge_depth = metal::min(metal::min(local_point_2.x, shadow_4.bounds.size.x - local_point_2.x), metal::min(local_point_2.y, shadow_4.bounds.size.y - local_point_2.y));
                bool _e56 = figma_has_corner_candidate(local_point_2, shadow_4.bounds.size, prepared_5.horizontal_reaches, prepared_5.vertical_reaches);
                if (!(_e56)) {
                    local_35 = edge_depth >= blur_limit;
                } else {
                    local_35 = false;
                }
                bool _e62 = local_35;
                if (_e62) {
                    coverage_3 = 1.0;
                } else {
                    float _e67 = prepared_corner_signed_distance(position_14, shadow_4.bounds, shadow_4.corner_radii, shadow_4.corner_smoothing, prepared_5);
                    float _e69 = gaussian_signed_distance_coverage(_e67, shadow_4.blur_radius);
                    coverage_3 = _e69;
                }
            }
        } else {
            float _e70 = blurred_shadow_coverage(shadow_4, position_14);
            coverage_3 = _e70;
        }
    }
    bool _e72 = is_enabled(shadow_4.inset);
    if (_e72) {
        float _e73 = coverage_3;
        float _e79 = prepared_corner_signed_distance(position_14, shadow_4.element_bounds, shadow_4.element_corner_radii, shadow_4.corner_smoothing, prepared_element);
        float _e80 = antialiased_coverage(_e79);
        coverage_3 = (1.0 - _e73) * _e80;
    }
    float _e82 = coverage_3;
    return _e82;
}

ShadowVertexData prepare_shadow_vertex(
    uint vertex_id_6,
    uint instance_id_2,
    Shadow shadow_5,
    constant GlobalUniforms& GLOBALS
) {
    Bounds _e3 = shadow_geometry(shadow_5);
    RectangleVertex _e4 = rectangle_vertex(vertex_id_6, _e3, GLOBALS);
    Paint _e6 = shadow_paint(shadow_5);
    PreparedPaint _e7 = prepare_paint(_e6);
    metal::float4 _e11 = clip_distances(_e4.viewport_position, shadow_5.content_mask.bounds);
    return ShadowVertexData {_e4.clip_position, _e7, instance_id_2, {}, _e11};
}

struct vertex_shadowInput {
};
struct vertex_shadowOutput {
    metal::float4 position [[position]];
    metal::float4 paint_solid [[user(loc0), flat]];
    metal::float4 paint_color0_ [[user(loc1), flat]];
    metal::float4 paint_color1_ [[user(loc2), flat]];
    uint shadow_id [[user(loc3), flat]];
    metal::float4 clip_distances [[user(loc4), center_perspective]];
};
vertex vertex_shadowOutput vertex_shadow(
  uint vertex_id [[vertex_id]]
, uint instance_id [[instance_id]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& SHADOWS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    Shadow shadow_6 = SHADOWS[static_cast<uint>(instance_id)];
    ShadowVertexData _e6 = prepare_shadow_vertex(vertex_id, instance_id, shadow_6, GLOBALS);
    const auto _tmp = ShadowVarying {_e6.position, _e6.paint.solid, _e6.paint.color0_, _e6.paint.color1_, _e6.shadow_id, {}, _e6.clip_distances};
    return vertex_shadowOutput { _tmp.position, _tmp.paint_solid, _tmp.paint_color0_, _tmp.paint_color1_, _tmp.shadow_id, _tmp.clip_distances };
}


struct fragment_shadowInput {
    metal::float4 paint_solid [[user(loc0), flat]];
    metal::float4 paint_color0_ [[user(loc1), flat]];
    metal::float4 paint_color1_ [[user(loc2), flat]];
    uint shadow_id [[user(loc3), flat]];
    metal::float4 clip_distances [[user(loc4), center_perspective]];
};
struct fragment_shadowOutput {
    metal::float4 member_1 [[color(0)]];
};
fragment fragment_shadowOutput fragment_shadow(
  fragment_shadowInput varyings_1 [[stage_in]]
, metal::float4 position_15 [[position]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& SHADOWS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const ShadowVarying input = { position_15, varyings_1.paint_solid, varyings_1.paint_color0_, varyings_1.paint_color1_, varyings_1.shadow_id, {}, varyings_1.clip_distances };
    bool _e2 = is_clipped(input.clip_distances);
    if (_e2) {
        metal::float4 _e3 = transparent();
        return fragment_shadowOutput { _e3 };
    }
    Shadow shadow_7 = SHADOWS[static_cast<uint>(input.shadow_id)];
    Paint _e9 = shadow_paint(shadow_7);
    PreparedPaint _e15 = PreparedPaint_new(input.paint_solid, input.paint_color0_, input.paint_color1_);
    metal::float4 _e16 = paint_color(_e9, input.position.xy, _e15);
    float _e19 = shadow_coverage(shadow_7, input.position.xy);
    float _e23 = ContentMask_alpha(shadow_7.content_mask, input.position.xy);
    metal::float4 _e25 = blend_color(_e16, _e19 * _e23, GLOBALS);
    return fragment_shadowOutput { _e25 };
}


struct vertex_smoothed_shadowInput {
};
struct vertex_smoothed_shadowOutput {
    metal::float4 position [[position]];
    metal::float4 paint_solid [[user(loc0), flat]];
    metal::float4 paint_color0_ [[user(loc1), flat]];
    metal::float4 paint_color1_ [[user(loc2), flat]];
    uint shadow_id [[user(loc3), flat]];
    metal::float4 clip_distances [[user(loc4), center_perspective]];
    metal::float4 horizontal_corner_reaches [[user(loc5), flat]];
    metal::float4 vertical_corner_reaches [[user(loc6), flat]];
    metal::float4 element_horizontal_corner_reaches [[user(loc7), flat]];
    metal::float4 element_vertical_corner_reaches [[user(loc8), flat]];
    metal::float4 smoothing_factors [[user(loc9), flat]];
};
vertex vertex_smoothed_shadowOutput vertex_smoothed_shadow(
  uint vertex_id_1 [[vertex_id]]
, uint instance_id_1 [[instance_id]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& SHADOWS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    Shadow shadow_8 = SHADOWS[static_cast<uint>(instance_id_1)];
    ShadowVertexData _e6 = prepare_shadow_vertex(vertex_id_1, instance_id_1, shadow_8, GLOBALS);
    PreparedCorners _e12 = prepare_corners(shadow_8.bounds.size, shadow_8.corner_radii, shadow_8.corner_smoothing, false);
    PreparedCorners _e18 = prepare_corners(shadow_8.element_bounds.size, shadow_8.element_corner_radii, shadow_8.corner_smoothing, false);
    const auto _tmp = SmoothedShadowVarying {_e6.position, _e6.paint.solid, _e6.paint.color0_, _e6.paint.color1_, _e6.shadow_id, {}, _e6.clip_distances, _e12.horizontal_reaches, _e12.vertical_reaches, _e18.horizontal_reaches, _e18.vertical_reaches, _e12.smoothing_factors};
    return vertex_smoothed_shadowOutput { _tmp.position, _tmp.paint_solid, _tmp.paint_color0_, _tmp.paint_color1_, _tmp.shadow_id, _tmp.clip_distances, _tmp.horizontal_corner_reaches, _tmp.vertical_corner_reaches, _tmp.element_horizontal_corner_reaches, _tmp.element_vertical_corner_reaches, _tmp.smoothing_factors };
}


struct fragment_smoothed_shadowInput {
    metal::float4 paint_solid [[user(loc0), flat]];
    metal::float4 paint_color0_ [[user(loc1), flat]];
    metal::float4 paint_color1_ [[user(loc2), flat]];
    uint shadow_id [[user(loc3), flat]];
    metal::float4 clip_distances [[user(loc4), center_perspective]];
    metal::float4 horizontal_corner_reaches [[user(loc5), flat]];
    metal::float4 vertical_corner_reaches [[user(loc6), flat]];
    metal::float4 element_horizontal_corner_reaches [[user(loc7), flat]];
    metal::float4 element_vertical_corner_reaches [[user(loc8), flat]];
    metal::float4 smoothing_factors [[user(loc9), flat]];
};
struct fragment_smoothed_shadowOutput {
    metal::float4 member_3 [[color(0)]];
};
fragment fragment_smoothed_shadowOutput fragment_smoothed_shadow(
  fragment_smoothed_shadowInput varyings_3 [[stage_in]]
, metal::float4 position_16 [[position]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& SHADOWS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const SmoothedShadowVarying input_1 = { position_16, varyings_3.paint_solid, varyings_3.paint_color0_, varyings_3.paint_color1_, varyings_3.shadow_id, {}, varyings_3.clip_distances, varyings_3.horizontal_corner_reaches, varyings_3.vertical_corner_reaches, varyings_3.element_horizontal_corner_reaches, varyings_3.element_vertical_corner_reaches, varyings_3.smoothing_factors };
    bool _e2 = is_clipped(input_1.clip_distances);
    if (_e2) {
        metal::float4 _e3 = transparent();
        return fragment_smoothed_shadowOutput { _e3 };
    }
    Shadow shadow_9 = SHADOWS[static_cast<uint>(input_1.shadow_id)];
    float _e21 = smoothed_shadow_coverage(shadow_9, input_1.position.xy, PreparedCorners {input_1.horizontal_corner_reaches, input_1.vertical_corner_reaches, input_1.smoothing_factors, 0.0}, PreparedCorners {input_1.element_horizontal_corner_reaches, input_1.element_vertical_corner_reaches, input_1.smoothing_factors, 0.0});
    Paint _e22 = shadow_paint(shadow_9);
    PreparedPaint _e28 = PreparedPaint_new(input_1.paint_solid, input_1.paint_color0_, input_1.paint_color1_);
    metal::float4 _e29 = paint_color(_e22, input_1.position.xy, _e28);
    float _e33 = ContentMask_alpha(shadow_9.content_mask, input_1.position.xy);
    metal::float4 _e35 = blend_color(_e29, _e21 * _e33, GLOBALS);
    return fragment_smoothed_shadowOutput { _e35 };
}
