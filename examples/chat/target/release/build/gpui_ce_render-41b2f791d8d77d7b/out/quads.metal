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
struct Quad {
    uint order;
    BorderStyle border_style;
    float border_dashed_length;
    float border_dashed_gap;
    Bounds bounds;
    ContentMask content_mask;
    Background background;
    Background border_color;
    Corners corner_radii;
    Edges border_widths;
    float corner_smoothing;
    uint padding;
};
typedef Quad type_14[1];
struct QuadGeometry {
    metal::float2 point;
    metal::float2 center_to_point;
    float corner_radius;
    char _pad3[4];
    metal::float2 corner_center_to_point;
    metal::float2 reduced_border;
    metal::float2 straight_border_inner_corner_to_point;
    bool near_rounded_corner;
    bool unrounded;
    char _pad8[6];
};
struct BorderDistances {
    float outer;
    float inner;
};
struct DashPosition {
    float position;
    float perimeter;
    float velocity;
};
struct RoundedDashLayout {
    Edges side_velocities;
    Corners corner_velocities;
    float right_start;
    float bottom_right_start;
    float bottom_left_start;
    float left_start;
    float top_left_start;
    float perimeter;
};
struct QuadVertexData {
    metal::float4 position;
    PreparedPaint border;
    uint quad_id;
    char _pad3[12];
    metal::float4 clip_distances;
    PreparedPaint fill;
};
struct QuadVarying {
    metal::float4 position;
    metal::float4 border_solid;
    uint quad_id;
    char _pad3[12];
    metal::float4 clip_distances;
    metal::float4 fill_solid;
    metal::float4 fill_color0_;
    metal::float4 fill_color1_;
    metal::float4 border_color0_;
    metal::float4 border_color1_;
};
struct SmoothedQuadVarying {
    metal::float4 position;
    metal::float4 border_solid;
    uint quad_id;
    char _pad3[12];
    metal::float4 clip_distances;
    metal::float4 fill_solid;
    metal::float4 fill_color0_;
    metal::float4 fill_color1_;
    metal::float4 horizontal_corner_reaches;
    metal::float4 vertical_corner_reaches;
    metal::float4 corner_lengths;
    metal::float4 smoothing_factors;
    float superellipse_power;
    char _pad12[12];
    metal::float4 border_color0_;
    metal::float4 border_color1_;
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
constant float DEFINITELY_OUTSIDE_INNER_BORDER = -1.0;

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
    bool local_4 = {};
    bool local_5 = {};
    bool local_6 = {};
    if (corners.top_left == 0.0) {
        local_4 = corners.top_right == 0.0;
    } else {
        local_4 = false;
    }
    bool _e10 = local_4;
    if (_e10) {
        local_5 = corners.bottom_right == 0.0;
    } else {
        local_5 = false;
    }
    bool _e17 = local_5;
    if (_e17) {
        local_6 = corners.bottom_left == 0.0;
    } else {
        local_6 = false;
    }
    bool _e24 = local_6;
    return _e24;
}

bool Edges_1is_zero(
    Edges edges
) {
    bool local_7 = {};
    bool local_8 = {};
    bool local_9 = {};
    if (edges.top == 0.0) {
        local_7 = edges.right == 0.0;
    } else {
        local_7 = false;
    }
    bool _e10 = local_7;
    if (_e10) {
        local_8 = edges.bottom == 0.0;
    } else {
        local_8 = false;
    }
    bool _e17 = local_8;
    if (_e17) {
        local_9 = edges.left == 0.0;
    } else {
        local_9 = false;
    }
    bool _e24 = local_9;
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
    metal::float4 color_2,
    float coverage
) {
    float alpha_5 = color_2.w * coverage;
    return metal::float4(color_2.x * alpha_5, color_2.y * alpha_5, color_2.z * alpha_5, alpha_5);
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
    metal::float3 color_3
) {
    return metal::dot(color_3, REC_601_LUMA_WEIGHTS);
}

float light_on_dark_contrast(
    float enhanced_contrast,
    metal::float3 color_4
) {
    float _e4 = color_brightness(color_4);
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
    metal::float3 color_5,
    float enhanced_contrast_factor,
    metal::float4 gamma_ratios_1
) {
    float _e4 = light_on_dark_contrast(enhanced_contrast_factor, color_5);
    float _e5 = _1enhance_contrast_f32_(sample, _e4);
    float _e6 = color_brightness(color_5);
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
    metal::float3 color_6,
    float enhanced_contrast_factor_1,
    metal::float4 gamma_ratios_3
) {
    float _e4 = light_on_dark_contrast(enhanced_contrast_factor_1, color_6);
    metal::float3 _e5 = _1enhance_contrast_vec3f(sample_1, _e4);
    metal::float3 _e6 = _2apply_alpha_correction_vec3f(_e5, color_6, gamma_ratios_3);
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
    metal::float4 color_7
) {
    metal::float3 _e2 = linear_to_srgb(color_7.xyz);
    return metal::float4(_e2.x, _e2.y, _e2.z, color_7.w);
}

metal::float4 srgba_to_linear(
    metal::float4 color_8
) {
    metal::float3 _e2 = srgb_to_linear(color_8.xyz);
    return metal::float4(_e2.x, _e2.y, _e2.z, color_8.w);
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
    metal::float4 color_9
) {
    metal::float3 linear_srgb = metal::pow(color_9.xyz, metal::float3(2.2, 2.2, 2.2));
    metal::float3 cone_response = LINEAR_SRGB_TO_CONE_RESPONSE * linear_srgb;
    metal::float3 cube_root = metal::float3(0.33333334, 0.33333334, 0.33333334);
    metal::float3 oklab = CONE_RESPONSE_TO_OKLAB * metal::pow(cone_response, cube_root);
    return metal::float4(oklab.x, oklab.y, oklab.z, color_9.w);
}

metal::float4 oklab_to_linear_srgb(
    metal::float4 color_10
) {
    metal::float3 cone_root = OKLAB_TO_CONE_RESPONSE * color_10.xyz;
    metal::float3 cone_response_1 = (cone_root * cone_root) * cone_root;
    metal::float3 linear_srgb_1 = CONE_RESPONSE_TO_LINEAR_SRGB * cone_response_1;
    metal::float3 srgb_1 = metal::pow(linear_srgb_1, metal::float3(0.45454547, 0.45454547, 0.45454547));
    return metal::float4(srgb_1.x, srgb_1.y, srgb_1.z, color_10.w);
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
    metal::float4 color_11,
    float alpha_factor,
    constant GlobalUniforms& GLOBALS
) {
    float alpha_7 = color_11.w * alpha_factor;
    ShaderBool _e6 = GLOBALS.premultiplied_alpha;
    bool _e7 = is_enabled(_e6);
    float multiplier = _e7 ? alpha_7 : 1.0;
    return metal::float4(color_11.x * multiplier, color_11.y * multiplier, color_11.z * multiplier, alpha_7);
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
    metal::float4 color_12 = {};
    float _e3 = linear_gradient_ratio(paint_2, position_7);
    metal::float4 ratio4_ = metal::float4(_e3, _e3, _e3, _e3);
    metal::float4 interpolated = metal::mix(prepared_1.color0_, prepared_1.color1_, ratio4_);
    metal::float4 _e8 = transparent();
    color_12 = _e8;
    if (paint_2.background.color_space == ColorSpace_Oklab) {
        metal::float4 _e14 = oklab_to_linear_srgb(interpolated);
        color_12 = _e14;
    } else {
        metal::float4 _e15 = srgba_to_linear(interpolated);
        color_12 = _e15;
    }
    metal::float4 _e16 = color_12;
    metal::float4 _e17 = gradient_dither(position_7);
    return _e16 + _e17;
}

metal::float4 slash_pattern_color(
    Paint paint_3,
    metal::float2 position_8,
    metal::float4 solid_1
) {
    metal::float4 color_13 = {};
    float encoded = paint_3.background.gradient_angle_or_pattern_height;
    float pattern_width = (encoded / PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
    float pattern_interval = metal::fmod(encoded, PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
    float pattern_height = pattern_width + pattern_interval;
    float period_1 = pattern_height * 0.70710677;
    metal::float2x2 rotation = metal::float2x2(metal::float2(0.70710677, -0.70710677), metal::float2(0.70710677, 0.70710677));
    float pattern = metal::fmod((rotation * (position_8 - paint_3.bounds.origin)).x, period_1);
    float distance_3 = metal::min(pattern, period_1 - pattern) - ((period_1 * (pattern_width / pattern_height)) / 2.0);
    color_13 = solid_1;
    float _e38 = color_13.w;
    float _e39 = antialiased_coverage(distance_3);
    color_13.w = _e38 * _e39;
    metal::float4 _e41 = color_13;
    return _e41;
}

metal::float4 checkerboard_color(
    Paint paint_4,
    metal::float2 position_9,
    metal::float4 solid_2
) {
    metal::float4 color_14 = {};
    metal::float2 relative = position_9 - paint_4.bounds.origin;
    float square_size = paint_4.background.gradient_angle_or_pattern_height;
    float colored = metal::fmod(metal::floor(relative.x / square_size) + metal::floor(relative.y / square_size), 2.0);
    color_14 = solid_2;
    float _e19 = color_14.w;
    color_14.w = _e19 * metal::saturate(colored);
    metal::float4 _e22 = color_14;
    return _e22;
}

metal::float4 paint_color(
    Paint paint_5,
    metal::float2 position_10,
    PreparedPaint prepared_2
) {
    metal::float4 color_15 = {};
    color_15 = prepared_2.solid;
    switch(paint_5.background.tag) {
        case 0u: {
            color_15 = prepared_2.solid;
            break;
        }
        case 1u: {
            metal::float4 _e8 = linear_gradient_color(paint_5, position_10, prepared_2);
            color_15 = _e8;
            break;
        }
        case 2u: {
            metal::float4 _e10 = slash_pattern_color(paint_5, position_10, prepared_2.solid);
            color_15 = _e10;
            break;
        }
        case 3u: {
            metal::float4 _e12 = checkerboard_color(paint_5, position_10, prepared_2.solid);
            color_15 = _e12;
            break;
        }
        default: {
            break;
        }
    }
    metal::float4 _e13 = color_15;
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
    bool local_10 = {};
    bool local_11 = {};
    bool local_12 = {};
    bool local_13 = {};
    bool local_14 = {};
    bool local_15 = {};
    bool local_16 = {};
    bool local_17 = {};
    bool local_18 = {};
    bool local_19 = {};
    metal::float4 _e3 = corner_values(corner_radii_3);
    metal::float4 radii_6 = metal::max(_e3, metal::float4(0.0, 0.0, 0.0, 0.0));
    metal::float4 _e10 = normalized_superellipse_reaches(corner_radii_3, corner_smoothing_2);
    if (corner_smoothing_2 > 0.0) {
        local_10 = size.x > 0.0;
    } else {
        local_10 = false;
    }
    bool _e19 = local_10;
    if (_e19) {
        local_11 = size.y > 0.0;
    } else {
        local_11 = false;
    }
    bool _e26 = local_11;
    if (_e26) {
        if (!((radii_6.x > 0.0))) {
            local_13 = radii_6.y > 0.0;
        } else {
            local_13 = true;
        }
        bool _e39 = local_13;
        if (!(_e39)) {
            local_14 = radii_6.z > 0.0;
        } else {
            local_14 = true;
        }
        bool _e47 = local_14;
        if (!(_e47)) {
            local_15 = radii_6.w > 0.0;
        } else {
            local_15 = true;
        }
        bool _e55 = local_15;
        local_12 = _e55;
    } else {
        local_12 = false;
    }
    bool _e57 = local_12;
    if (_e57) {
        local_16 = (_e10.x + _e10.y) <= size.x;
    } else {
        local_16 = false;
    }
    bool _e66 = local_16;
    if (_e66) {
        local_17 = (_e10.y + _e10.z) <= size.y;
    } else {
        local_17 = false;
    }
    bool _e75 = local_17;
    if (_e75) {
        local_18 = (_e10.z + _e10.w) <= size.x;
    } else {
        local_18 = false;
    }
    bool _e84 = local_18;
    if (_e84) {
        local_19 = (_e10.w + _e10.x) <= size.y;
    } else {
        local_19 = false;
    }
    bool _e93 = local_19;
    return _e93;
}

float normalized_superellipse_signed_distance_from_corner(
    metal::float2 corner_to_point,
    float corner_radius_2,
    float corner_smoothing_3,
    float power
) {
    bool local_20 = {};
    bool local_21 = {};
    float extent = metal::max(corner_radius_2, 0.0) * (1.0 + metal::clamp(corner_smoothing_3, 0.0, 1.0));
    metal::float2 corner_center_to_point_1 = corner_to_point + metal::float2(extent);
    if (!((extent <= 0.0))) {
        local_20 = corner_center_to_point_1.x <= 0.0;
    } else {
        local_20 = true;
    }
    bool _e23 = local_20;
    if (!(_e23)) {
        local_21 = corner_center_to_point_1.y <= 0.0;
    } else {
        local_21 = true;
    }
    bool _e31 = local_21;
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
    bool local_22 = {};
    bool local_23 = {};
    bool local_24 = {};
    float half_short_side = 0.5 * metal::min(size_1.x, size_1.y);
    if (reaches.x <= half_short_side) {
        local_22 = reaches.y <= half_short_side;
    } else {
        local_22 = false;
    }
    bool _e14 = local_22;
    if (_e14) {
        local_23 = reaches.z <= half_short_side;
    } else {
        local_23 = false;
    }
    bool _e20 = local_23;
    if (_e20) {
        local_24 = reaches.w <= half_short_side;
    } else {
        local_24 = false;
    }
    bool _e26 = local_24;
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
    bool local_25 = {};
    bool local_26 = {};
    bool local_27 = {};
    bool local_28 = {};
    bool local_29 = {};
    if (!((horizontal_extent <= FIGMA_EPSILON))) {
        local_25 = vertical_extent <= FIGMA_EPSILON;
    } else {
        local_25 = true;
    }
    bool _e13 = local_25;
    if (_e13) {
        return false;
    }
    switch(corner) {
        case 0u: {
            if (point_4.x <= horizontal_extent) {
                local_26 = point_4.y <= vertical_extent;
            } else {
                local_26 = false;
            }
            bool _e22 = local_26;
            return _e22;
        }
        case 1u: {
            if ((size_3.x - point_4.x) <= horizontal_extent) {
                local_27 = point_4.y <= vertical_extent;
            } else {
                local_27 = false;
            }
            bool _e32 = local_27;
            return _e32;
        }
        case 2u: {
            if ((size_3.x - point_4.x) <= horizontal_extent) {
                local_28 = (size_3.y - point_4.y) <= vertical_extent;
            } else {
                local_28 = false;
            }
            bool _e44 = local_28;
            return _e44;
        }
        default: {
            if (point_4.x <= horizontal_extent) {
                local_29 = (size_3.y - point_4.y) <= vertical_extent;
            } else {
                local_29 = false;
            }
            bool _e54 = local_29;
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
    bool local_30 = {};
    float vertical_budget = {};
    bool local_31 = {};
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
                local_30 = horizontal_radius != 0.0;
            } else {
                local_30 = true;
            }
            bool _e119 = local_30;
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
                local_31 = vertical_radius != 0.0;
            } else {
                local_31 = true;
            }
            bool _e150 = local_31;
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
    metal::float4 horizontal_1 = metal::max(horizontal_budgets, zero);
    metal::float4 vertical = metal::max(vertical_budgets, zero);
    metal::float4 _e11 = corner_values(corner_radii_8);
    metal::float4 radii_7 = metal::min(metal::max(_e11, zero), metal::min(horizontal_1, vertical));
    metal::float4 reaches_3 = radii_7 * (1.0 + metal::clamp(corner_smoothing_8, 0.0, 1.0));
    return FigmaCornerExtents {metal::min(reaches_3, horizontal_1), metal::min(reaches_3, vertical)};
}

PreparedCorners prepare_corners(
    metal::float2 size_6,
    Corners radii_3,
    float smoothing,
    bool allow_superellipse
) {
    PreparedCorners prepared_3 = {};
    bool local_32 = {};
    metal::float4 _e4 = corner_values(radii_3);
    prepared_3 = PreparedCorners {_e4, _e4, metal::float4(0.0, 1.0, 0.0, 0.0), 0.0};
    if (smoothing <= 0.0) {
        PreparedCorners _e15 = prepared_3;
        return _e15;
    }
    if (allow_superellipse) {
        bool _e18 = can_use_normalized_superellipse(size_6, radii_3, smoothing);
        local_32 = _e18;
    } else {
        local_32 = false;
    }
    bool _e20 = local_32;
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
    bool local_33 = {};
    bool local_34 = {};
    SignedDistanceSample sample_2 = {};
    metal::float2 z = corner_to_point_1 + metal::float2(params.horizontal.p, params.vertical.p);
    if (!((params.radius <= FIGMA_EPSILON))) {
        local_33 = z.x <= 0.0;
    } else {
        local_33 = true;
    }
    bool _e18 = local_33;
    if (!(_e18)) {
        local_34 = z.y <= 0.0;
    } else {
        local_34 = true;
    }
    bool _e26 = local_34;
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
    bool local_35 = {};
    float center = metal::clamp(end_t, 0.0, 1.0) / 2.0;
    if (!((center <= 0.0))) {
        local_35 = params_1.smoothing <= 0.0;
    } else {
        local_35 = true;
    }
    bool _e17 = local_35;
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
    bool local_36 = {};
    bool local_37 = {};
    bool local_38 = {};
    bool _e7 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.x, vertical_extents_1.x, 0u);
    if (!(_e7)) {
        bool _e14 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.y, vertical_extents_1.y, 1u);
        local_36 = _e14;
    } else {
        local_36 = true;
    }
    bool _e16 = local_36;
    if (!(_e16)) {
        bool _e23 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.z, vertical_extents_1.z, 2u);
        local_37 = _e23;
    } else {
        local_37 = true;
    }
    bool _e25 = local_37;
    if (!(_e25)) {
        bool _e32 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.w, vertical_extents_1.w, 3u);
        local_38 = _e32;
    } else {
        local_38 = true;
    }
    bool _e34 = local_38;
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

QuadGeometry quad_geometry(
    Quad quad_,
    metal::float2 position_12
) {
    bool local_39 = {};
    metal::float2 _e3 = Bounds_1half_size(quad_.bounds);
    metal::float2 point_12 = position_12 - quad_.bounds.origin;
    metal::float2 center_to_point_3 = point_12 - _e3;
    float _e9 = pick_corner_radius(center_to_point_3, quad_.corner_radii);
    metal::float2 corner_to_point_3 = metal::abs(center_to_point_3) - _e3;
    metal::float2 corner_center_to_point_2 = corner_to_point_3 + metal::float2(_e9);
    metal::float2 border_2 = metal::float2((center_to_point_3.x < 0.0) ? quad_.border_widths.left : quad_.border_widths.right, (center_to_point_3.y < 0.0) ? quad_.border_widths.top : quad_.border_widths.bottom);
    metal::float2 reduced_border = metal::float2((border_2.x == 0.0) ? -0.5 : border_2.x, (border_2.y == 0.0) ? -0.5 : border_2.y);
    if (corner_center_to_point_2.x >= 0.0) {
        local_39 = corner_center_to_point_2.y >= 0.0;
    } else {
        local_39 = false;
    }
    bool _e54 = local_39;
    bool _e56 = Corners_1is_zero(quad_.corner_radii);
    return QuadGeometry {point_12, center_to_point_3, _e9, {}, corner_center_to_point_2, reduced_border, corner_to_point_3 + reduced_border, _e54, _e56};
}

bool is_unaffected_background(
    QuadGeometry geometry
) {
    bool local_40 = {};
    bool local_41 = {};
    if (geometry.straight_border_inner_corner_to_point.x < -0.5) {
        local_40 = geometry.straight_border_inner_corner_to_point.y < -0.5;
    } else {
        local_40 = false;
    }
    bool _e12 = local_40;
    if (_e12) {
        local_41 = !(geometry.near_rounded_corner);
    } else {
        local_41 = false;
    }
    bool _e18 = local_41;
    return _e18;
}

float inner_border_signed_distance(
    QuadGeometry geometry_1,
    float outer_signed_distance
) {
    bool local_42 = {};
    bool local_43 = {};
    if (!((geometry_1.corner_center_to_point.x <= 0.0))) {
        local_42 = geometry_1.corner_center_to_point.y <= 0.0;
    } else {
        local_42 = true;
    }
    bool _e14 = local_42;
    if (_e14) {
        return -(metal::max(geometry_1.straight_border_inner_corner_to_point.x, geometry_1.straight_border_inner_corner_to_point.y));
    }
    if (!((geometry_1.straight_border_inner_corner_to_point.x > 0.0))) {
        local_43 = geometry_1.straight_border_inner_corner_to_point.y > 0.0;
    } else {
        local_43 = true;
    }
    bool _e33 = local_43;
    if (_e33) {
        return DEFINITELY_OUTSIDE_INNER_BORDER;
    }
    if (geometry_1.reduced_border.x == geometry_1.reduced_border.y) {
        return -((outer_signed_distance + geometry_1.reduced_border.x));
    }
    metal::float2 ellipse_radii = metal::max(metal::float2(0.0, 0.0), metal::float2(geometry_1.corner_radius) - geometry_1.reduced_border);
    float _e53 = quarter_ellipse_signed_distance(geometry_1.corner_center_to_point, ellipse_radii);
    return _e53;
}

BorderDistances border_distances(
    QuadGeometry geometry_2
) {
    float _e3 = rounded_rectangle_signed_distance_from_corner(geometry_2.corner_center_to_point, geometry_2.corner_radius);
    float _e4 = inner_border_signed_distance(geometry_2, _e3);
    return BorderDistances {_e3, _e4};
}

float dash_period_per_border_width(
    Quad quad_1
) {
    return metal::max(quad_1.border_dashed_length, 0.0) + metal::max(quad_1.border_dashed_gap, 0.0);
}

float dash_velocity_1(
    float border_width,
    float period_per_border_width
) {
    bool local_44 = {};
    if (!((border_width <= 0.0))) {
        local_44 = period_per_border_width <= 0.0;
    } else {
        local_44 = true;
    }
    bool _e10 = local_44;
    if (_e10) {
        return 0.0;
    } else {
        return (1.0 / period_per_border_width) / border_width;
    }
}

DashPosition straight_dash_position(
    Quad quad_2,
    QuadGeometry geometry_3
) {
    bool horizontal_2 = geometry_3.corner_center_to_point.x < geometry_3.corner_center_to_point.y;
    float border_width_1 = horizontal_2 ? metal::max(quad_2.border_widths.bottom, quad_2.border_widths.top) : metal::max(quad_2.border_widths.right, quad_2.border_widths.left);
    float _e18 = dash_period_per_border_width(quad_2);
    float _e19 = dash_velocity_1(border_width_1, _e18);
    return DashPosition {(horizontal_2 ? geometry_3.point.x : geometry_3.point.y) * _e19, (horizontal_2 ? quad_2.bounds.size.x : quad_2.bounds.size.y) * _e19, _e19};
}

Edges side_dash_velocities(
    Edges border_widths,
    float period_per_border_width_1
) {
    float _e3 = dash_velocity_1(border_widths.top, period_per_border_width_1);
    float _e5 = dash_velocity_1(border_widths.right, period_per_border_width_1);
    float _e7 = dash_velocity_1(border_widths.bottom, period_per_border_width_1);
    float _e9 = dash_velocity_1(border_widths.left, period_per_border_width_1);
    return Edges {_e3, _e5, _e7, _e9};
}

Edges straight_side_dash_lengths(
    Bounds bounds_12,
    Corners radii_4,
    Edges velocities
) {
    return Edges {((bounds_12.size.x - radii_4.top_left) - radii_4.top_right) * velocities.top, ((bounds_12.size.y - radii_4.top_right) - radii_4.bottom_right) * velocities.right, ((bounds_12.size.x - radii_4.bottom_right) - radii_4.bottom_left) * velocities.bottom, ((bounds_12.size.y - radii_4.bottom_left) - radii_4.top_left) * velocities.left};
}

Corners corner_dash_velocities(
    Edges side_velocities
) {
    float _e3 = corner_dash_velocity(side_velocities.top, side_velocities.left);
    float _e6 = corner_dash_velocity(side_velocities.top, side_velocities.right);
    float _e9 = corner_dash_velocity(side_velocities.bottom, side_velocities.right);
    float _e12 = corner_dash_velocity(side_velocities.bottom, side_velocities.left);
    return Corners {_e3, _e6, _e9, _e12};
}

Corners corner_dash_lengths(
    Corners radii_5,
    Corners velocities_1
) {
    return Corners {(radii_5.top_left * 1.5707964) * velocities_1.top_left, (radii_5.top_right * 1.5707964) * velocities_1.top_right, (radii_5.bottom_right * 1.5707964) * velocities_1.bottom_right, (radii_5.bottom_left * 1.5707964) * velocities_1.bottom_left};
}

RoundedDashLayout perimeter_dash_layout(
    Edges side_velocities_1,
    Edges side_lengths,
    Corners corner_velocities,
    metal::float4 corner_lengths
) {
    float right_start = side_lengths.top + corner_lengths.y;
    float bottom_right_start = right_start + side_lengths.right;
    float bottom_left_start = (bottom_right_start + corner_lengths.z) + side_lengths.bottom;
    float left_start = bottom_left_start + corner_lengths.w;
    float top_left_start = left_start + side_lengths.left;
    return RoundedDashLayout {side_velocities_1, corner_velocities, right_start, bottom_right_start, bottom_left_start, left_start, top_left_start, top_left_start + corner_lengths.x};
}

RoundedDashLayout rounded_dash_layout(
    Quad quad_3
) {
    float _e2 = dash_period_per_border_width(quad_3);
    Edges _e3 = side_dash_velocities(quad_3.border_widths, _e2);
    Edges _e6 = straight_side_dash_lengths(quad_3.bounds, quad_3.corner_radii, _e3);
    Corners _e7 = corner_dash_velocities(_e3);
    Corners _e9 = corner_dash_lengths(quad_3.corner_radii, _e7);
    metal::float4 _e10 = corner_values(_e9);
    RoundedDashLayout _e11 = perimeter_dash_layout(_e3, _e6, _e7, _e10);
    return _e11;
}

metal::float4 smoothed_corner_lengths(
    Quad quad_4,
    PreparedCorners prepared_5
) {
    bool local_45 = {};
    if (!((quad_4.border_style != BorderStyle_Dashed))) {
        local_45 = quad_4.corner_smoothing <= 0.0;
    } else {
        local_45 = true;
    }
    bool _e12 = local_45;
    if (_e12) {
        metal::float4 _e14 = corner_values(quad_4.corner_radii);
        return _e14 * 1.5707964;
    }
    metal::float4 _e18 = corner_values(quad_4.corner_radii);
    FigmaCornerParams _e25 = figma_corner_params(_e18.x, prepared_5.horizontal_reaches.x, prepared_5.vertical_reaches.x, prepared_5.smoothing_factors);
    float _e26 = figma_corner_length(_e25);
    FigmaCornerParams _e33 = figma_corner_params(_e18.y, prepared_5.horizontal_reaches.y, prepared_5.vertical_reaches.y, prepared_5.smoothing_factors);
    float _e34 = figma_corner_length(_e33);
    FigmaCornerParams _e41 = figma_corner_params(_e18.z, prepared_5.horizontal_reaches.z, prepared_5.vertical_reaches.z, prepared_5.smoothing_factors);
    float _e42 = figma_corner_length(_e41);
    FigmaCornerParams _e49 = figma_corner_params(_e18.w, prepared_5.horizontal_reaches.w, prepared_5.vertical_reaches.w, prepared_5.smoothing_factors);
    float _e50 = figma_corner_length(_e49);
    return metal::float4(_e26, _e34, _e42, _e50);
}

RoundedDashLayout smoothed_dash_layout(
    Quad quad_5,
    PreparedCorners prepared_6,
    metal::float4 corner_lengths_1
) {
    float _e4 = dash_period_per_border_width(quad_5);
    Edges _e5 = side_dash_velocities(quad_5.border_widths, _e4);
    Edges side_lengths_1 = Edges {((quad_5.bounds.size.x - prepared_6.horizontal_reaches.x) - prepared_6.horizontal_reaches.y) * _e5.top, ((quad_5.bounds.size.y - prepared_6.vertical_reaches.y) - prepared_6.vertical_reaches.z) * _e5.right, ((quad_5.bounds.size.x - prepared_6.horizontal_reaches.z) - prepared_6.horizontal_reaches.w) * _e5.bottom, ((quad_5.bounds.size.y - prepared_6.vertical_reaches.w) - prepared_6.vertical_reaches.x) * _e5.left};
    Corners _e51 = corner_dash_velocities(_e5);
    metal::float4 _e52 = corner_values(_e51);
    RoundedDashLayout _e54 = perimeter_dash_layout(_e5, side_lengths_1, _e51, corner_lengths_1 * _e52);
    return _e54;
}

float smoothed_dashed_border_alpha(
    Quad quad_6,
    QuadGeometry geometry_4,
    FigmaRectangleSample rectangle_sample,
    PreparedCorners prepared_7,
    metal::float4 corner_lengths_2,
    metal::float2 border_1,
    metal::float2 straight_border_inner_corner_to_point
) {
    float dash_position = 0.0;
    float dash_velocity_2 = 0.0;
    bool local_46 = {};
    bool horizontal = {};
    float straight_width = {};
    float _e7 = dash_period_per_border_width(quad_6);
    if (_e7 <= 0.0) {
        return 1.0;
    }
    RoundedDashLayout _e11 = smoothed_dash_layout(quad_6, prepared_7, corner_lengths_2);
    if (rectangle_sample.corner != FIGMA_NO_CORNER) {
        local_46 = rectangle_sample.signed_distance.segment != FIGMA_SEGMENT_STRAIGHT;
    } else {
        local_46 = false;
    }
    bool _e26 = local_46;
    if (_e26) {
        uint corner_idx = rectangle_sample.corner;
        metal::float4 _e29 = corner_values(quad_6.corner_radii);
        float corner_length = corner_lengths_2[corner_idx];
        FigmaCornerParams _e37 = figma_corner_params(_e29[corner_idx], prepared_7.horizontal_reaches[corner_idx], prepared_7.vertical_reaches[corner_idx], prepared_7.smoothing_factors);
        float _e39 = figma_corner_progress(_e37, rectangle_sample.signed_distance, corner_length);
        switch(corner_idx) {
            case 0u: {
                dash_velocity_2 = _e11.corner_velocities.top_left;
                float _e44 = dash_velocity_2;
                dash_position = _e11.top_left_start + ((corner_length - _e39) * _e44);
                break;
            }
            case 1u: {
                dash_velocity_2 = _e11.corner_velocities.top_right;
                float _e50 = dash_velocity_2;
                dash_position = _e11.right_start + (_e39 * _e50);
                break;
            }
            case 2u: {
                dash_velocity_2 = _e11.corner_velocities.bottom_right;
                float _e57 = dash_velocity_2;
                dash_position = _e11.bottom_right_start + ((corner_length - _e39) * _e57);
                break;
            }
            default: {
                dash_velocity_2 = _e11.corner_velocities.bottom_left;
                float _e63 = dash_velocity_2;
                dash_position = _e11.bottom_left_start + (_e39 * _e63);
                break;
            }
        }
    } else {
        horizontal = straight_border_inner_corner_to_point.x < straight_border_inner_corner_to_point.y;
        bool _e72 = horizontal;
        straight_width = _e72 ? border_1.y : border_1.x;
        float _e75 = straight_width;
        if (_e75 <= 0.0) {
            bool _e78 = horizontal;
            horizontal = !(_e78);
            bool _e82 = horizontal;
            straight_width = _e82 ? border_1.y : border_1.x;
        }
        bool _e84 = horizontal;
        if (_e84) {
            if (geometry_4.center_to_point.y < 0.0) {
                dash_velocity_2 = _e11.side_velocities.top;
                float _e96 = dash_velocity_2;
                dash_position = (geometry_4.point.x - prepared_7.horizontal_reaches.x) * _e96;
            } else {
                dash_velocity_2 = _e11.side_velocities.bottom;
                float _e106 = dash_velocity_2;
                dash_position = _e11.bottom_left_start - ((geometry_4.point.x - prepared_7.horizontal_reaches.w) * _e106);
            }
        } else {
            if (geometry_4.center_to_point.x < 0.0) {
                dash_velocity_2 = _e11.side_velocities.left;
                float _e121 = dash_velocity_2;
                dash_position = _e11.top_left_start - ((geometry_4.point.y - prepared_7.vertical_reaches.x) * _e121);
            } else {
                dash_velocity_2 = _e11.side_velocities.right;
                float _e132 = dash_velocity_2;
                dash_position = _e11.right_start + ((geometry_4.point.y - prepared_7.vertical_reaches.y) * _e132);
            }
        }
    }
    float dash_length_1 = metal::max(quad_6.border_dashed_length, 0.0) / _e7;
    if (_e11.perimeter >= 1.0) {
        float period_2 = _e11.perimeter / metal::floor(_e11.perimeter);
        float _e146 = dash_position;
        float _e147 = dash_velocity_2;
        float _e148 = dash_coverage(_e146, period_2, dash_length_1, _e147);
        return _e148;
    } else {
        return 1.0;
    }
}

DashPosition rounded_dash_position(
    Quad quad_7,
    QuadGeometry geometry_5
) {
    Corners radii_8 = quad_7.corner_radii;
    RoundedDashLayout _e3 = rounded_dash_layout(quad_7);
    bool horizontal_3 = geometry_5.corner_center_to_point.x < geometry_5.corner_center_to_point.y;
    bool on_right = geometry_5.center_to_point.x >= 0.0;
    bool on_bottom = geometry_5.center_to_point.y >= 0.0;
    float top_position = (geometry_5.point.x - quad_7.corner_radii.top_left) * _e3.side_velocities.top;
    float right_position = _e3.right_start + ((geometry_5.point.y - quad_7.corner_radii.top_right) * _e3.side_velocities.right);
    float bottom_position = _e3.bottom_left_start - ((geometry_5.point.x - quad_7.corner_radii.bottom_left) * _e3.side_velocities.bottom);
    float left_position = _e3.top_left_start - ((geometry_5.point.y - quad_7.corner_radii.top_left) * _e3.side_velocities.left);
    float horizontal_position_1 = on_bottom ? bottom_position : top_position;
    float vertical_position_1 = on_right ? right_position : left_position;
    float side_position = horizontal_3 ? horizontal_position_1 : vertical_position_1;
    float horizontal_velocity = on_bottom ? _e3.side_velocities.bottom : _e3.side_velocities.top;
    float vertical_velocity = on_right ? _e3.side_velocities.right : _e3.side_velocities.left;
    float side_velocity = horizontal_3 ? horizontal_velocity : vertical_velocity;
    if (geometry_5.near_rounded_corner) {
        float corner_position = metal::atan2(geometry_5.corner_center_to_point.y, geometry_5.corner_center_to_point.x) * geometry_5.corner_radius;
        float top_right_position = _e3.right_start - (corner_position * _e3.corner_velocities.top_right);
        float bottom_right_position = _e3.bottom_right_start + (corner_position * _e3.corner_velocities.bottom_right);
        float bottom_left_position = _e3.left_start - (corner_position * _e3.corner_velocities.bottom_left);
        float top_left_position = _e3.top_left_start + (corner_position * _e3.corner_velocities.top_left);
        float right_position_1 = on_bottom ? bottom_right_position : top_right_position;
        float left_position_1 = on_bottom ? bottom_left_position : top_left_position;
        float position_13 = on_right ? right_position_1 : left_position_1;
        float right_velocity = on_bottom ? _e3.corner_velocities.bottom_right : _e3.corner_velocities.top_right;
        float left_velocity = on_bottom ? _e3.corner_velocities.bottom_left : _e3.corner_velocities.top_left;
        return DashPosition {position_13, _e3.perimeter, on_right ? right_velocity : left_velocity};
    }
    return DashPosition {side_position, _e3.perimeter, side_velocity};
}

float dashed_border_alpha(
    Quad quad_8,
    QuadGeometry geometry_6
) {
    DashPosition dash = DashPosition {0.0, 0.0, 0.0};
    bool local_47 = {};
    float _e2 = dash_period_per_border_width(quad_8);
    if (_e2 <= 0.0) {
        return 1.0;
    }
    if (geometry_6.unrounded) {
        DashPosition _e12 = straight_dash_position(quad_8, geometry_6);
        dash = _e12;
    } else {
        DashPosition _e13 = rounded_dash_position(quad_8, geometry_6);
        dash = _e13;
    }
    float dash_length_2 = metal::max(quad_8.border_dashed_length, 0.0) / _e2;
    float _e19 = dash.perimeter;
    float perimeter = _e19 - (geometry_6.unrounded ? dash_length_2 : 0.0);
    if (perimeter >= 1.0) {
        float period_3 = perimeter / metal::floor(perimeter);
        float _e29 = dash.position;
        float _e31 = dash.velocity;
        float _e32 = dash_coverage(_e29, period_3, dash_length_2, _e31);
        return _e32;
    } else {
        if (geometry_6.unrounded) {
            local_47 = perimeter > dash_length_2;
        } else {
            local_47 = false;
        }
        bool _e38 = local_47;
        if (_e38) {
            float _e40 = dash.position;
            float _e42 = dash.velocity;
            float _e43 = dash_coverage(_e40, perimeter, dash_length_2, _e42);
            return _e43;
        } else {
            return 1.0;
        }
    }
}

QuadVertexData prepare_quad_vertex(
    uint vertex_id_6,
    uint instance_id_2,
    Quad quad_9,
    constant GlobalUniforms& GLOBALS
) {
    RectangleVertex _e4 = rectangle_vertex(vertex_id_6, quad_9.bounds, GLOBALS);
    Paint _e8 = Paint_new(quad_9.border_color, quad_9.bounds);
    PreparedPaint _e9 = prepare_paint(_e8);
    metal::float4 _e13 = clip_distances(_e4.viewport_position, quad_9.content_mask.bounds);
    Paint _e16 = Paint_new(quad_9.background, quad_9.bounds);
    PreparedPaint _e17 = prepare_paint(_e16);
    return QuadVertexData {_e4.clip_position, _e9, instance_id_2, {}, _e13, _e17};
}

struct vertex_quadInput {
};
struct vertex_quadOutput {
    metal::float4 position [[position]];
    metal::float4 border_solid [[user(loc0), flat]];
    uint quad_id [[user(loc1), flat]];
    metal::float4 clip_distances [[user(loc2), center_perspective]];
    metal::float4 fill_solid [[user(loc3), flat]];
    metal::float4 fill_color0_ [[user(loc4), flat]];
    metal::float4 fill_color1_ [[user(loc5), flat]];
    metal::float4 border_color0_ [[user(loc6), flat]];
    metal::float4 border_color1_ [[user(loc7), flat]];
};
vertex vertex_quadOutput vertex_quad(
  uint vertex_id [[vertex_id]]
, uint instance_id [[instance_id]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& QUADS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    Quad quad_10 = QUADS[static_cast<uint>(instance_id)];
    QuadVertexData _e6 = prepare_quad_vertex(vertex_id, instance_id, quad_10, GLOBALS);
    const auto _tmp = QuadVarying {_e6.position, _e6.border.solid, _e6.quad_id, {}, _e6.clip_distances, _e6.fill.solid, _e6.fill.color0_, _e6.fill.color1_, _e6.border.color0_, _e6.border.color1_};
    return vertex_quadOutput { _tmp.position, _tmp.border_solid, _tmp.quad_id, _tmp.clip_distances, _tmp.fill_solid, _tmp.fill_color0_, _tmp.fill_color1_, _tmp.border_color0_, _tmp.border_color1_ };
}


struct fragment_quadInput {
    metal::float4 border_solid [[user(loc0), flat]];
    uint quad_id [[user(loc1), flat]];
    metal::float4 clip_distances [[user(loc2), center_perspective]];
    metal::float4 fill_solid [[user(loc3), flat]];
    metal::float4 fill_color0_ [[user(loc4), flat]];
    metal::float4 fill_color1_ [[user(loc5), flat]];
    metal::float4 border_color0_ [[user(loc6), flat]];
    metal::float4 border_color1_ [[user(loc7), flat]];
};
struct fragment_quadOutput {
    metal::float4 member_1 [[color(0)]];
};
fragment fragment_quadOutput fragment_quad(
  fragment_quadInput varyings_1 [[stage_in]]
, metal::float4 position_14 [[position]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& QUADS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const QuadVarying input = { position_14, varyings_1.border_solid, varyings_1.quad_id, {}, varyings_1.clip_distances, varyings_1.fill_solid, varyings_1.fill_color0_, varyings_1.fill_color1_, varyings_1.border_color0_, varyings_1.border_color1_ };
    bool local = {};
    metal::float4 color = {};
    metal::float4 border_color = {};
    bool _e2 = is_clipped(input.clip_distances);
    if (_e2) {
        metal::float4 _e3 = transparent();
        return fragment_quadOutput { _e3 };
    }
    Quad quad_11 = QUADS[static_cast<uint>(input.quad_id)];
    float _e12 = ContentMask_alpha(quad_11.content_mask, input.position.xy);
    Paint _e15 = Paint_new(quad_11.background, quad_11.bounds);
    PreparedPaint _e21 = PreparedPaint_new(input.fill_solid, input.fill_color0_, input.fill_color1_);
    metal::float4 _e22 = paint_color(_e15, input.position.xy, _e21);
    bool _e24 = Edges_1is_zero(quad_11.border_widths);
    if (_e24) {
        bool _e28 = Corners_1is_zero(quad_11.corner_radii);
        local = _e28;
    } else {
        local = false;
    }
    bool _e30 = local;
    if (_e30) {
        metal::float4 _e31 = blend_color(_e22, _e12, GLOBALS);
        return fragment_quadOutput { _e31 };
    }
    QuadGeometry _e34 = quad_geometry(quad_11, input.position.xy);
    bool _e35 = is_unaffected_background(_e34);
    if (_e35) {
        metal::float4 _e36 = blend_color(_e22, _e12, GLOBALS);
        return fragment_quadOutput { _e36 };
    }
    BorderDistances _e37 = border_distances(_e34);
    color = _e22;
    if (metal::max(_e37.inner, _e37.outer) < PIXEL_ANTIALIAS_RADIUS) {
        Paint _e46 = Paint_new(quad_11.border_color, quad_11.bounds);
        PreparedPaint _e52 = PreparedPaint_new(input.border_solid, input.border_color0_, input.border_color1_);
        metal::float4 _e53 = paint_color(_e46, input.position.xy, _e52);
        border_color = _e53;
        if (quad_11.border_style == BorderStyle_Dashed) {
            float _e59 = border_color.w;
            float _e60 = dashed_border_alpha(quad_11, _e34);
            border_color.w = _e59 * _e60;
        }
        metal::float4 _e62 = border_color;
        metal::float4 _e63 = over(_e22, _e62);
        float _e65 = antialiased_coverage(_e37.inner);
        color = metal::mix(_e22, _e63, metal::float4(_e65, _e65, _e65, _e65));
    }
    metal::float4 _e68 = color;
    float _e70 = antialiased_coverage(_e37.outer);
    metal::float4 _e72 = blend_color(_e68, _e70 * _e12, GLOBALS);
    return fragment_quadOutput { _e72 };
}


struct vertex_smoothed_quadInput {
};
struct vertex_smoothed_quadOutput {
    metal::float4 position [[position]];
    metal::float4 border_solid [[user(loc0), flat]];
    uint quad_id [[user(loc1), flat]];
    metal::float4 clip_distances [[user(loc2), center_perspective]];
    metal::float4 fill_solid [[user(loc3), flat]];
    metal::float4 fill_color0_ [[user(loc4), flat]];
    metal::float4 fill_color1_ [[user(loc5), flat]];
    metal::float4 horizontal_corner_reaches [[user(loc6), flat]];
    metal::float4 vertical_corner_reaches [[user(loc7), flat]];
    metal::float4 corner_lengths [[user(loc8), flat]];
    metal::float4 smoothing_factors [[user(loc9), flat]];
    float superellipse_power [[user(loc10), flat]];
    metal::float4 border_color0_ [[user(loc11), flat]];
    metal::float4 border_color1_ [[user(loc12), flat]];
};
vertex vertex_smoothed_quadOutput vertex_smoothed_quad(
  uint vertex_id_1 [[vertex_id]]
, uint instance_id_1 [[instance_id]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& QUADS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    bool local_1 = {};
    Quad quad_12 = QUADS[static_cast<uint>(instance_id_1)];
    QuadVertexData _e6 = prepare_quad_vertex(vertex_id_1, instance_id_1, quad_12, GLOBALS);
    if (quad_12.border_style == BorderStyle_Solid) {
        bool _e17 = Edges_1is_zero(quad_12.border_widths);
        local_1 = _e17;
    } else {
        local_1 = false;
    }
    bool _e19 = local_1;
    PreparedCorners _e20 = prepare_corners(quad_12.bounds.size, quad_12.corner_radii, quad_12.corner_smoothing, _e19);
    metal::float4 _e34 = smoothed_corner_lengths(quad_12, _e20);
    const auto _tmp = SmoothedQuadVarying {_e6.position, _e6.border.solid, _e6.quad_id, {}, _e6.clip_distances, _e6.fill.solid, _e6.fill.color0_, _e6.fill.color1_, _e20.horizontal_reaches, _e20.vertical_reaches, _e34, _e20.smoothing_factors, _e20.superellipse_power, {}, _e6.border.color0_, _e6.border.color1_};
    return vertex_smoothed_quadOutput { _tmp.position, _tmp.border_solid, _tmp.quad_id, _tmp.clip_distances, _tmp.fill_solid, _tmp.fill_color0_, _tmp.fill_color1_, _tmp.horizontal_corner_reaches, _tmp.vertical_corner_reaches, _tmp.corner_lengths, _tmp.smoothing_factors, _tmp.superellipse_power, _tmp.border_color0_, _tmp.border_color1_ };
}


struct fragment_smoothed_quadInput {
    metal::float4 border_solid [[user(loc0), flat]];
    uint quad_id [[user(loc1), flat]];
    metal::float4 clip_distances [[user(loc2), center_perspective]];
    metal::float4 fill_solid [[user(loc3), flat]];
    metal::float4 fill_color0_ [[user(loc4), flat]];
    metal::float4 fill_color1_ [[user(loc5), flat]];
    metal::float4 horizontal_corner_reaches [[user(loc6), flat]];
    metal::float4 vertical_corner_reaches [[user(loc7), flat]];
    metal::float4 corner_lengths [[user(loc8), flat]];
    metal::float4 smoothing_factors [[user(loc9), flat]];
    float superellipse_power [[user(loc10), flat]];
    metal::float4 border_color0_ [[user(loc11), flat]];
    metal::float4 border_color1_ [[user(loc12), flat]];
};
struct fragment_smoothed_quadOutput {
    metal::float4 member_3 [[color(0)]];
};
fragment fragment_smoothed_quadOutput fragment_smoothed_quad(
  fragment_smoothed_quadInput varyings_3 [[stage_in]]
, metal::float4 position_15 [[position]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_14 const& QUADS [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const SmoothedQuadVarying input_1 = { position_15, varyings_3.border_solid, varyings_3.quad_id, {}, varyings_3.clip_distances, varyings_3.fill_solid, varyings_3.fill_color0_, varyings_3.fill_color1_, varyings_3.horizontal_corner_reaches, varyings_3.vertical_corner_reaches, varyings_3.corner_lengths, varyings_3.smoothing_factors, varyings_3.superellipse_power, {}, varyings_3.border_color0_, varyings_3.border_color1_ };
    metal::float2 border = {};
    bool local_2 = {};
    bool local_3 = {};
    float inner = 0.0;
    metal::float4 color_1 = {};
    metal::float4 border_color_1 = {};
    bool _e2 = is_clipped(input_1.clip_distances);
    if (_e2) {
        metal::float4 _e3 = transparent();
        return fragment_smoothed_quadOutput { _e3 };
    }
    Quad quad_13 = QUADS[static_cast<uint>(input_1.quad_id)];
    float _e12 = ContentMask_alpha(quad_13.content_mask, input_1.position.xy);
    Paint _e15 = Paint_new(quad_13.background, quad_13.bounds);
    PreparedPaint _e21 = PreparedPaint_new(input_1.fill_solid, input_1.fill_color0_, input_1.fill_color1_);
    metal::float4 _e22 = paint_color(_e15, input_1.position.xy, _e21);
    PreparedCorners prepared_8 = PreparedCorners {input_1.horizontal_corner_reaches, input_1.vertical_corner_reaches, input_1.smoothing_factors, input_1.superellipse_power};
    bool _e29 = Edges_1is_zero(quad_13.border_widths);
    if (_e29) {
        float _e35 = prepared_corner_signed_distance(input_1.position.xy, quad_13.bounds, quad_13.corner_radii, quad_13.corner_smoothing, prepared_8);
        float _e36 = antialiased_coverage(_e35);
        metal::float4 _e38 = blend_color(_e22, _e36 * _e12, GLOBALS);
        return fragment_smoothed_quadOutput { _e38 };
    }
    QuadGeometry _e41 = quad_geometry(quad_13, input_1.position.xy);
    FigmaRectangleSample _e49 = figma_smooth_rectangle_sample(input_1.position.xy, quad_13.bounds, quad_13.corner_radii, prepared_8.horizontal_reaches, prepared_8.vertical_reaches, prepared_8.smoothing_factors);
    border = metal::float2((_e41.center_to_point.x < 0.0) ? quad_13.border_widths.left : quad_13.border_widths.right, (_e41.center_to_point.y < 0.0) ? quad_13.border_widths.top : quad_13.border_widths.bottom);
    switch(_e49.corner) {
        case 0u: {
            border = metal::float2(quad_13.border_widths.left, quad_13.border_widths.top);
            break;
        }
        case 1u: {
            border = metal::float2(quad_13.border_widths.right, quad_13.border_widths.top);
            break;
        }
        case 2u: {
            border = metal::float2(quad_13.border_widths.right, quad_13.border_widths.bottom);
            break;
        }
        case 3u: {
            border = metal::float2(quad_13.border_widths.left, quad_13.border_widths.bottom);
            break;
        }
        default: {
            break;
        }
    }
    float _e92 = border.x;
    float _e95 = border.x;
    float _e100 = border.y;
    float _e103 = border.y;
    metal::float2 reduced_border_1 = metal::float2((_e95 == 0.0) ? -0.5 : _e92, (_e103 == 0.0) ? -0.5 : _e100);
    metal::float2 _e109 = Bounds_1half_size(quad_13.bounds);
    metal::float2 corner_to_point_4 = metal::abs(_e41.center_to_point) - _e109;
    metal::float2 straight_border_inner_corner_to_point_1 = corner_to_point_4 + reduced_border_1;
    bool near_curve = _e49.signed_distance.segment != FIGMA_SEGMENT_STRAIGHT;
    if (straight_border_inner_corner_to_point_1.x < -0.5) {
        local_2 = straight_border_inner_corner_to_point_1.y < -0.5;
    } else {
        local_2 = false;
    }
    bool _e127 = local_2;
    if (_e127) {
        local_3 = !(near_curve);
    } else {
        local_3 = false;
    }
    bool _e132 = local_3;
    if (_e132) {
        metal::float4 _e133 = blend_color(_e22, _e12, GLOBALS);
        return fragment_smoothed_quadOutput { _e133 };
    }
    float outer = _e49.signed_distance.distance;
    if (near_curve) {
        metal::float2 normal_5 = metal::abs(_e49.signed_distance.normal);
        float _e142 = border.x;
        float _e149 = border.y;
        metal::float2 active_sides = metal::float2((_e142 > 0.0) ? 1.0 : 0.0, (_e149 > 0.0) ? 1.0 : 0.0);
        metal::float2 _e156 = border;
        float effective_width = metal::length(_e156 * normal_5) - (PIXEL_ANTIALIAS_RADIUS * (1.0 - metal::length(active_sides * normal_5)));
        inner = -((outer + effective_width));
    } else {
        inner = -(metal::max(straight_border_inner_corner_to_point_1.x, straight_border_inner_corner_to_point_1.y));
    }
    color_1 = _e22;
    float _e173 = inner;
    if (metal::max(_e173, outer) < PIXEL_ANTIALIAS_RADIUS) {
        Paint _e179 = Paint_new(quad_13.border_color, quad_13.bounds);
        PreparedPaint _e185 = PreparedPaint_new(input_1.border_solid, input_1.border_color0_, input_1.border_color1_);
        metal::float4 _e186 = paint_color(_e179, input_1.position.xy, _e185);
        border_color_1 = _e186;
        if (quad_13.border_style == BorderStyle_Dashed) {
            float _e192 = border_color_1.w;
            metal::float2 _e194 = border;
            float _e195 = smoothed_dashed_border_alpha(quad_13, _e41, _e49, prepared_8, input_1.corner_lengths, _e194, straight_border_inner_corner_to_point_1);
            border_color_1.w = _e192 * _e195;
        }
        metal::float4 _e197 = border_color_1;
        metal::float4 _e198 = over(_e22, _e197);
        float _e199 = inner;
        float _e200 = antialiased_coverage(_e199);
        color_1 = metal::mix(_e22, _e198, metal::float4(_e200, _e200, _e200, _e200));
    }
    metal::float4 _e203 = color_1;
    float _e204 = antialiased_coverage(outer);
    metal::float4 _e206 = blend_color(_e203, _e204 * _e12, GLOBALS);
    return fragment_smoothed_quadOutput { _e206 };
}
