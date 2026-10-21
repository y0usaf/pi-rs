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
struct Underline {
    uint order;
    uint padding;
    Bounds bounds;
    ContentMask content_mask;
    Hsla color;
    float thickness;
    ShaderBool wavy;
};
typedef Underline type_12[1];
struct UnderlineVarying {
    metal::float4 position;
    metal::float4 color;
    uint underline_id;
    char _pad3[12];
    metal::float4 clip_distances;
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

float ContentMask_alpha(
    ContentMask content_mask,
    metal::float2 position
) {
    float alpha = 1.0;
    Bounds bounds_7 = content_mask.bounds;
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
    uint vertex_id_1
) {
    return metal::float2(static_cast<float>(vertex_id_1 & 1u), 0.5 * static_cast<float>(vertex_id_1 & 2u));
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
    uint vertex_id_2,
    Bounds bounds_3,
    constant GlobalUniforms& GLOBALS
) {
    metal::float2 _e2 = rectangle_corner(vertex_id_2);
    metal::float2 _e3 = Bounds_position(bounds_3, _e2);
    metal::float4 _e4 = viewport_to_clip_position(_e3, GLOBALS);
    return RectangleVertex {_e2, _e3, _e4};
}

RectangleVertex transformed_rectangle_vertex(
    uint vertex_id_3,
    Bounds bounds_4,
    TransformationMatrix transform_1,
    constant GlobalUniforms& GLOBALS
) {
    metal::float2 _e3 = rectangle_corner(vertex_id_3);
    metal::float2 _e4 = Bounds_position(bounds_4, _e3);
    metal::float2 _e5 = TransformationMatrix_1transform_position(transform_1, _e4);
    metal::float4 _e6 = viewport_to_clip_position(_e5, GLOBALS);
    return RectangleVertex {_e3, _e5, _e6};
}

FullscreenVertex fullscreen_vertex(
    uint vertex_id_4
) {
    metal::float2 texture_coordinates = metal::float2(static_cast<float>((vertex_id_4 << 1u) & 2u), static_cast<float>(vertex_id_4 & 2u));
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
    float distance_1 = metal::min(pattern, period_1 - pattern) - ((period_1 * (pattern_width / pattern_height)) / 2.0);
    color_11 = solid_1;
    float _e38 = color_11.w;
    float _e39 = antialiased_coverage(distance_1);
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

float wavy_underline_coverage(
    Underline underline,
    metal::float2 position_12
) {
    metal::float2 normalized_position = ((position_12 - underline.bounds.origin) / metal::float2(underline.bounds.size.y)) - metal::float2(0.0, 0.5);
    float angular_frequency = (6.2831855 * underline.thickness) / underline.bounds.size.y;
    float amplitude = (underline.thickness * UNDERLINE_WAVE_HEIGHT_RATIO) / underline.bounds.size.y;
    float wave_height = metal::sin(normalized_position.x * angular_frequency) * amplitude;
    float wave_slope = (metal::cos(normalized_position.x * angular_frequency) * amplitude) * angular_frequency;
    float signed_distance_2 = ((normalized_position.y - wave_height) / metal::sqrt(1.0 + (wave_slope * wave_slope))) * underline.bounds.size.y;
    float stroke_distance = metal::abs(signed_distance_2) - (underline.thickness * 0.5);
    float _e53 = antialiased_coverage(stroke_distance);
    return _e53;
}

struct vertex_underlineInput {
};
struct vertex_underlineOutput {
    metal::float4 position [[position]];
    metal::float4 color [[user(loc0), flat]];
    uint underline_id [[user(loc1), flat]];
    metal::float4 clip_distances [[user(loc3), center_perspective]];
};
vertex vertex_underlineOutput vertex_underline(
  uint vertex_id [[vertex_id]]
, uint instance_id [[instance_id]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_12 const& UNDERLINES [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    Underline underline_1 = UNDERLINES[static_cast<uint>(instance_id)];
    RectangleVertex _e7 = rectangle_vertex(vertex_id, underline_1.bounds, GLOBALS);
    metal::float4 _e10 = hsla_to_rgba(underline_1.color);
    metal::float4 _e14 = clip_distances(_e7.viewport_position, underline_1.content_mask.bounds);
    const auto _tmp = UnderlineVarying {_e7.clip_position, _e10, instance_id, {}, _e14};
    return vertex_underlineOutput { _tmp.position, _tmp.color, _tmp.underline_id, _tmp.clip_distances };
}


struct fragment_underlineInput {
    metal::float4 color [[user(loc0), flat]];
    uint underline_id [[user(loc1), flat]];
    metal::float4 clip_distances [[user(loc3), center_perspective]];
};
struct fragment_underlineOutput {
    metal::float4 member_1 [[color(0)]];
};
fragment fragment_underlineOutput fragment_underline(
  fragment_underlineInput varyings_1 [[stage_in]]
, metal::float4 position_13 [[position]]
, constant GlobalUniforms& GLOBALS [[buffer(0)]]
, device type_12 const& UNDERLINES [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const UnderlineVarying input = { position_13, varyings_1.color, varyings_1.underline_id, {}, varyings_1.clip_distances };
    bool _e2 = is_clipped(input.clip_distances);
    if (_e2) {
        metal::float4 _e3 = transparent();
        return fragment_underlineOutput { _e3 };
    }
    Underline underline_2 = UNDERLINES[static_cast<uint>(input.underline_id)];
    bool _e10 = is_enabled(underline_2.wavy);
    if (!(_e10)) {
        float _e16 = ContentMask_alpha(underline_2.content_mask, input.position.xy);
        metal::float4 _e17 = blend_color(input.color, _e16, GLOBALS);
        return fragment_underlineOutput { _e17 };
    }
    float _e21 = wavy_underline_coverage(underline_2, input.position.xy);
    float _e25 = ContentMask_alpha(underline_2.content_mask, input.position.xy);
    metal::float4 _e27 = blend_color(input.color, _e21 * _e25, GLOBALS);
    return fragment_underlineOutput { _e27 };
}
