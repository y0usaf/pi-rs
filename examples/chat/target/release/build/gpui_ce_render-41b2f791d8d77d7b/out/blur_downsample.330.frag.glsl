#version 330 core
struct GlobalUniforms {
    vec2 viewport_size;
    uint premultiplied_alpha;
    uint padding;
};
struct FontRasterizationUniforms {
    vec4 gamma_ratios;
    float grayscale_enhanced_contrast;
    float subpixel_enhanced_contrast;
    uint uses_blue_green_red_subpixel_order;
    uint padding;
};
struct Bounds {
    vec2 origin;
    vec2 size;
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
struct Background {
    uint tag;
    uint color_space;
    Hsla solid;
    float gradient_angle_or_pattern_height;
    LinearColorStop colors[2];
    uint padding;
};
struct AtlasTextureId {
    uint index;
    uint kind;
};
struct AtlasBounds {
    ivec2 origin;
    ivec2 size;
};
struct AtlasTile {
    AtlasTextureId texture_id;
    uint tile_id;
    uint padding;
    AtlasBounds bounds;
};
struct TransformationMatrix {
    mat2x2 rotation_scale;
    vec2 translation;
};
struct RectangleVertex {
    vec2 unit_position;
    vec2 viewport_position;
    vec4 clip_position;
};
struct FullscreenVertex {
    vec2 texture_coordinates;
    vec4 clip_position;
};
struct Paint {
    Background background;
    Bounds bounds;
};
struct PreparedPaint {
    vec4 solid;
    vec4 color0_;
    vec4 color1_;
};
struct SignedDistanceSample {
    float distance_;
    vec2 normal;
    float path_t;
    uint segment;
};
struct FigmaCornerLayout {
    vec4 horizontal_budgets;
    vec4 vertical_budgets;
};
struct FigmaCornerExtents {
    vec4 horizontal;
    vec4 vertical;
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
    vec2 point;
    vec2 tangent;
    float distance_;
    float path_t;
};
struct FigmaRectangleSample {
    SignedDistanceSample signed_distance;
    uint corner;
};
struct PreparedCorners {
    vec4 horizontal_reaches;
    vec4 vertical_reaches;
    vec4 smoothing_factors;
    float superellipse_power;
};
struct BlurUniforms {
    Bounds bounds;
    Bounds content_mask;
    Corners corner_radii;
    vec2 direction;
    float standard_deviation;
    float opacity;
    uint sample_count;
    float sample_step;
    uint composite_clip;
    uint downsample_mode;
    vec2 source_size;
    vec2 target_size;
    float corner_smoothing;
    uint padding0_;
    uint padding1_;
    uint padding2_;
};
struct BlurCompositeVertexData {
    vec4 position;
    vec2 texture_coordinates;
    vec4 clip_distances;
};
struct BlurVarying {
    vec4 position;
    vec2 texture_coordinates;
    vec4 clip_distances;
};
struct SmoothedBlurVarying {
    vec4 position;
    vec2 texture_coordinates;
    vec4 horizontal_corner_reaches;
    vec4 vertical_corner_reaches;
    vec4 clip_distances;
    vec4 smoothing_factors;
    float superellipse_power;
};
const float PI = 3.1415927;
const float HALF_TURN_DEGREES = 180.0;
const float FULL_TURN_DEGREES = 360.0;
const float CSS_GRADIENT_OFFSET_DEGREES = 90.0;
const float PIXEL_ANTIALIAS_RADIUS = 0.5;
const float GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS = 3.0;
const float PATTERN_COMPONENT_SCALE = 255.0;
const float PATTERN_PACKING_RADIX = 65535.0;
const float MIN_NORMALIZED_WEIGHT = 1e-5;
const float MIN_PATH_GRADIENT = 0.001;
const float UNDERLINE_WAVE_FREQUENCY = 2.0;
const float UNDERLINE_WAVE_HEIGHT_RATIO = 0.8;
const float GRADIENT_DITHER_SCALE = 0.618034;
const float GRADIENT_DITHER_RGB_STRENGTH = 0.007843138;
const float GRADIENT_DITHER_ALPHA_STRENGTH = 0.011764706;
const vec2 GRADIENT_DITHER_SEED_A = vec2(12.9898, 78.233);
const vec2 GRADIENT_DITHER_SEED_B = vec2(39.346, 11.135);
const float GRADIENT_DITHER_MULTIPLIER_A = 43758.547;
const float GRADIENT_DITHER_MULTIPLIER_B = 24634.635;
const float DIAGONAL_STRIPE_ANGLE = 0.7853982;
const vec3 REC_601_LUMA_WEIGHTS = vec3(0.3, 0.59, 0.11);
const vec3 LINEAR_RGB_LUMA_WEIGHTS = vec3(0.2126, 0.7152, 0.0722);
const float SRGB_DECODE_CUTOFF = 0.04045;
const float SRGB_ENCODE_CUTOFF = 0.0031308;
const float SRGB_TRANSFER_OFFSET = 0.055;
const float SRGB_TRANSFER_SCALE = 1.055;
const float SRGB_DECODE_EXPONENT = 2.4;
const float SRGB_LINEAR_SCALE = 12.92;
const float ERROR_FUNCTION_LINEAR_COEFFICIENT = 0.278393;
const float ERROR_FUNCTION_QUADRATIC_COEFFICIENT = 0.230389;
const float ERROR_FUNCTION_CUBIC_COEFFICIENT = 0.000972;
const float ERROR_FUNCTION_QUARTIC_COEFFICIENT = 0.078108;
const float DARK_TEXT_CONTRAST_SCALE = 4.0;
const float DARK_TEXT_BRIGHTNESS_CUTOFF = 0.75;
const mat3x3 LINEAR_SRGB_TO_CONE_RESPONSE = mat3x3(vec3(0.41222146, 0.2119035, 0.08830246), vec3(0.53633255, 0.6806995, 0.28171885), vec3(0.051445995, 0.10739696, 0.6299787));
const mat3x3 CONE_RESPONSE_TO_OKLAB = mat3x3(vec3(0.21045426, 1.9779985, 0.025904037), vec3(0.7936178, -2.4285922, 0.78277177), vec3(-0.004072047, 0.4505939, -0.80867577));
const mat3x3 OKLAB_TO_CONE_RESPONSE = mat3x3(vec3(1.0, 1.0, 1.0), vec3(0.39633778, -0.105561346, -0.08948418), vec3(0.21580376, -0.06385417, -1.2914855));
const mat3x3 CONE_RESPONSE_TO_LINEAR_SRGB = mat3x3(vec3(4.0767417, -1.268438, -0.0041960864), vec3(-3.3077116, 2.6097574, -0.7034186), vec3(0.23096994, -0.34131938, 1.7076147));
const uint ShaderBool_Disabled = 0u;
const uint ShaderBool_Enabled = 1u;
const uint BackgroundTag_Solid = 0u;
const uint BackgroundTag_LinearGradient = 1u;
const uint BackgroundTag_PatternSlash = 2u;
const uint BackgroundTag_Checkerboard = 3u;
const uint ColorSpace_Srgb = 0u;
const uint ColorSpace_Oklab = 1u;
const uint BorderStyle_Solid = 0u;
const uint BorderStyle_Dashed = 1u;
const uint SurfaceColorFormat_Rgba = 0u;
const uint SurfaceColorFormat_Yuv = 1u;
const uint BlurCompositeClip_None = 0u;
const uint BlurCompositeClip_RoundedBounds = 1u;
const uint DownsampleMode_Copy = 0u;
const uint DownsampleMode_HalfResolution = 1u;
const float SUPERELLIPSE_DIAGONAL_INSET = 0.29289323;
const uint FIGMA_SEGMENT_STRAIGHT = 0u;
const uint FIGMA_SEGMENT_FIRST_CUBIC = 1u;
const uint FIGMA_SEGMENT_ARC = 2u;
const uint FIGMA_SEGMENT_SECOND_CUBIC = 3u;
const uint FIGMA_NO_CORNER = 4u;
const float FIGMA_EPSILON = 1e-6;

layout(std140) uniform BlurUniforms_block_0Fragment { BlurUniforms _group_1_binding_0_fs; };

uniform sampler2D _group_1_binding_1_fs;

smooth in vec2 _vs2fs_location0;
smooth in vec4 _vs2fs_location3;
layout(location = 0) out vec4 _fs2p_location0;

float ContentMask_alpha(ContentMask content_mask, vec2 position) {
    float alpha = 1.0;
    Bounds bounds_12 = content_mask.bounds;
    Edges fade = content_mask.fade_out;
    if ((fade.left > 0.0)) {
        float _e9 = alpha;
        alpha = (_e9 * clamp(((position.x - bounds_12.origin.x) / fade.left), 0.0, 1.0));
    }
    if ((fade.right > 0.0)) {
        float _e21 = alpha;
        alpha = (_e21 * clamp((((bounds_12.origin.x + bounds_12.size.x) - position.x) / fade.right), 0.0, 1.0));
    }
    if ((fade.top > 0.0)) {
        float _e36 = alpha;
        alpha = (_e36 * clamp(((position.y - bounds_12.origin.y) / fade.top), 0.0, 1.0));
    }
    if ((fade.bottom > 0.0)) {
        float _e48 = alpha;
        alpha = (_e48 * clamp((((bounds_12.origin.y + bounds_12.size.y) - position.y) / fade.bottom), 0.0, 1.0));
    }
    float _e60 = alpha;
    return _e60;
}

vec2 Bounds_position(Bounds bounds, vec2 unit_position) {
    return (bounds.origin + (unit_position * bounds.size));
}

vec2 Bounds_1half_size(Bounds bounds_1) {
    return (bounds_1.size / vec2(2.0));
}

vec2 Bounds_center(Bounds bounds_2) {
    vec2 _e2 = Bounds_1half_size(bounds_2);
    return (bounds_2.origin + _e2);
}

bool Corners_1is_zero(Corners corners) {
    bool local = false;
    bool local_1 = false;
    bool local_2 = false;
    if ((corners.top_left == 0.0)) {
        local = (corners.top_right == 0.0);
    } else {
        local = false;
    }
    bool _e10 = local;
    if (_e10) {
        local_1 = (corners.bottom_right == 0.0);
    } else {
        local_1 = false;
    }
    bool _e17 = local_1;
    if (_e17) {
        local_2 = (corners.bottom_left == 0.0);
    } else {
        local_2 = false;
    }
    bool _e24 = local_2;
    return _e24;
}

bool Edges_1is_zero(Edges edges) {
    bool local_3 = false;
    bool local_4 = false;
    bool local_5 = false;
    if ((edges.top == 0.0)) {
        local_3 = (edges.right == 0.0);
    } else {
        local_3 = false;
    }
    bool _e10 = local_3;
    if (_e10) {
        local_4 = (edges.bottom == 0.0);
    } else {
        local_4 = false;
    }
    bool _e17 = local_4;
    if (_e17) {
        local_5 = (edges.left == 0.0);
    } else {
        local_5 = false;
    }
    bool _e24 = local_5;
    return _e24;
}

vec2 TransformationMatrix_1transform_position(TransformationMatrix transform, vec2 position_1) {
    return ((transpose(transform.rotation_scale) * position_1) + transform.translation);
}

bool is_enabled(uint value) {
    return (value == ShaderBool_Enabled);
}

vec4 transparent() {
    return vec4(0.0, 0.0, 0.0, 0.0);
}

float antialiased_coverage(float signed_distance) {
    return clamp((PIXEL_ANTIALIAS_RADIUS - signed_distance), 0.0, 1.0);
}

vec4 premultiply(vec4 color, float coverage) {
    float alpha_5 = (color.w * coverage);
    return vec4((color.x * alpha_5), (color.y * alpha_5), (color.z * alpha_5), alpha_5);
}

vec2 atlas_texture_coordinates(vec2 unit_position_1, AtlasTile tile, uvec2 atlas_size) {
    vec2 tile_origin = vec2(float(tile.bounds.origin.x), float(tile.bounds.origin.y));
    vec2 tile_size = vec2(float(tile.bounds.size.x), float(tile.bounds.size.y));
    vec2 texture_size = vec2(float(atlas_size.x), float(atlas_size.y));
    return ((tile_origin + (unit_position_1 * tile_size)) / texture_size);
}

float color_brightness(vec3 color_1) {
    return dot(color_1, REC_601_LUMA_WEIGHTS);
}

float light_on_dark_contrast(float enhanced_contrast, vec3 color_2) {
    float _e4 = color_brightness(color_2);
    float darkness = clamp((DARK_TEXT_CONTRAST_SCALE * (DARK_TEXT_BRIGHTNESS_CUTOFF - _e4)), 0.0, 1.0);
    return (enhanced_contrast * darkness);
}

float _2apply_alpha_correction_f32_(float alpha_1, float brightness, vec4 gamma_ratios) {
    float brightness_adjustment = ((brightness * gamma_ratios.x) + gamma_ratios.y);
    float correction = ((brightness_adjustment * alpha_1) + ((brightness * gamma_ratios.z) + gamma_ratios.w));
    return (alpha_1 + ((alpha_1 * (1.0 - alpha_1)) * correction));
}

float _1enhance_contrast_f32_(float alpha_2, float contrast) {
    return ((alpha_2 * (contrast + 1.0)) / ((alpha_2 * contrast) + 1.0));
}

float apply_contrast_and_gamma_correction(float sample_, vec3 color_3, float enhanced_contrast_factor, vec4 gamma_ratios_1) {
    float _e4 = light_on_dark_contrast(enhanced_contrast_factor, color_3);
    float _e5 = _1enhance_contrast_f32_(sample_, _e4);
    float _e6 = color_brightness(color_3);
    float _e7 = _2apply_alpha_correction_f32_(_e5, _e6, gamma_ratios_1);
    return _e7;
}

vec3 _1enhance_contrast_vec3f(vec3 alpha_3, float contrast_1) {
    return ((alpha_3 * (contrast_1 + 1.0)) / ((alpha_3 * contrast_1) + vec3(1.0)));
}

vec3 _2apply_alpha_correction_vec3f(vec3 alpha_4, vec3 brightness_1, vec4 gamma_ratios_2) {
    vec3 brightness_adjustment_1 = ((brightness_1 * gamma_ratios_2.x) + vec3(gamma_ratios_2.y));
    vec3 correction_1 = ((brightness_adjustment_1 * alpha_4) + ((brightness_1 * gamma_ratios_2.z) + vec3(gamma_ratios_2.w)));
    return (alpha_4 + ((alpha_4 * (vec3(1.0) - alpha_4)) * correction_1));
}

vec3 apply_contrast_and_gamma_correction3_(vec3 sample_1, vec3 color_4, float enhanced_contrast_factor_1, vec4 gamma_ratios_3) {
    float _e4 = light_on_dark_contrast(enhanced_contrast_factor_1, color_4);
    vec3 _e5 = _1enhance_contrast_vec3f(sample_1, _e4);
    vec3 _e6 = _2apply_alpha_correction_vec3f(_e5, color_4, gamma_ratios_3);
    return _e6;
}

vec2 rectangle_corner(uint vertex_id_3) {
    return vec2(float((vertex_id_3 & 1u)), (0.5 * float((vertex_id_3 & 2u))));
}

FullscreenVertex fullscreen_vertex(uint vertex_id_6) {
    vec2 texture_coordinates_2 = vec2(float(((vertex_id_6 << 1u) & 2u)), float((vertex_id_6 & 2u)));
    return FullscreenVertex(texture_coordinates_2, vec4(((texture_coordinates_2.x * 2.0) - 1.0), (1.0 - (texture_coordinates_2.y * 2.0)), 0.0, 1.0));
}

vec4 clip_distances(vec2 position_3, Bounds clip_bounds) {
    vec2 distance_from_top_left = (position_3 - clip_bounds.origin);
    vec2 distance_from_bottom_right = ((clip_bounds.origin + clip_bounds.size) - position_3);
    return vec4(distance_from_top_left.x, distance_from_bottom_right.x, distance_from_top_left.y, distance_from_bottom_right.y);
}

bool is_clipped(vec4 distances) {
    return (min(min(distances.x, distances.y), min(distances.z, distances.w)) < 0.0);
}

vec4 unclipped_distances() {
    return vec4(1.0, 1.0, 1.0, 1.0);
}

vec3 srgb_to_linear(vec3 srgb) {
    bvec3 cutoff = bvec3((srgb.x < SRGB_DECODE_CUTOFF), (srgb.y < SRGB_DECODE_CUTOFF), (srgb.z < SRGB_DECODE_CUTOFF));
    vec3 higher = pow(((srgb + vec3(0.055)) / vec3(1.055)), vec3(2.4, 2.4, 2.4));
    return mix(higher, (srgb / vec3(12.92)), cutoff);
}

vec3 linear_to_srgb(vec3 linear) {
    bvec3 cutoff_1 = bvec3((linear.x < SRGB_ENCODE_CUTOFF), (linear.y < SRGB_ENCODE_CUTOFF), (linear.z < SRGB_ENCODE_CUTOFF));
    vec3 higher_1 = ((SRGB_TRANSFER_SCALE * pow(linear, vec3(0.41666666, 0.41666666, 0.41666666))) - vec3(0.055));
    return mix(higher_1, (linear * SRGB_LINEAR_SCALE), cutoff_1);
}

vec4 linear_to_srgba(vec4 color_5) {
    vec3 _e2 = linear_to_srgb(color_5.xyz);
    return vec4(_e2.x, _e2.y, _e2.z, color_5.w);
}

vec4 srgba_to_linear(vec4 color_6) {
    vec3 _e2 = srgb_to_linear(color_6.xyz);
    return vec4(_e2.x, _e2.y, _e2.z, color_6.w);
}

vec3 hue_to_unit_rgb(float hue) {
    vec3 phases = fract(vec3(hue, (hue + 0.6666667), (hue + 0.33333334)));
    return clamp((abs(((phases * 6.0) - vec3(3.0))) - vec3(1.0)), vec3(0.0), vec3(1.0));
}

vec4 hsla_to_rgba(Hsla hsla) {
    float chroma = (hsla.s * (1.0 - abs(((2.0 * hsla.l) - 1.0))));
    vec3 _e12 = hue_to_unit_rgb(hsla.h);
    vec3 red_green_blue = (((_e12 - vec3(0.5)) * chroma) + vec3(hsla.l));
    return vec4(red_green_blue.x, red_green_blue.y, red_green_blue.z, hsla.a);
}

vec4 linear_srgb_to_oklab(vec4 color_7) {
    vec3 linear_srgb = pow(color_7.xyz, vec3(2.2, 2.2, 2.2));
    vec3 cone_response = (LINEAR_SRGB_TO_CONE_RESPONSE * linear_srgb);
    vec3 cube_root = vec3(0.33333334, 0.33333334, 0.33333334);
    vec3 oklab = (CONE_RESPONSE_TO_OKLAB * pow(cone_response, cube_root));
    return vec4(oklab.x, oklab.y, oklab.z, color_7.w);
}

vec4 oklab_to_linear_srgb(vec4 color_8) {
    vec3 cone_root = (OKLAB_TO_CONE_RESPONSE * color_8.xyz);
    vec3 cone_response_1 = ((cone_root * cone_root) * cone_root);
    vec3 linear_srgb_1 = (CONE_RESPONSE_TO_LINEAR_SRGB * cone_response_1);
    vec3 srgb_1 = pow(linear_srgb_1, vec3(0.45454547, 0.45454547, 0.45454547));
    return vec4(srgb_1.x, srgb_1.y, srgb_1.z, color_8.w);
}

vec4 over(vec4 below, vec4 above) {
    float alpha_6 = (above.w + (below.w * (1.0 - above.w)));
    if ((alpha_6 == 0.0)) {
        vec4 _e11 = transparent();
        return _e11;
    }
    vec3 red_green_blue_1 = (((above.xyz * above.w) + ((below.xyz * below.w) * (1.0 - above.w))) / vec3(alpha_6));
    return vec4(red_green_blue_1.x, red_green_blue_1.y, red_green_blue_1.z, alpha_6);
}

float gaussian(float position_4, float standard_deviation) {
    float variance = (standard_deviation * standard_deviation);
    return (exp((-((position_4 * position_4)) / (2.0 * variance))) / (2.5066283 * standard_deviation));
}

vec2 approximate_error_function(vec2 value_1) {
    vec2 signs = sign(value_1);
    vec2 absolute_value = abs(value_1);
    vec2 polynomial = (vec2(1.0) + (absolute_value * (vec2(0.278393) + (absolute_value * (vec2(0.230389) + (absolute_value * (vec2(0.000972) + (absolute_value * ERROR_FUNCTION_QUARTIC_COEFFICIENT))))))));
    vec2 polynomial_squared = (polynomial * polynomial);
    return (signs - (signs / (polynomial_squared * polynomial_squared)));
}

float integrated_rounded_rectangle_coverage(float horizontal_position, float vertical_position, float standard_deviation_1, float corner_radius, vec2 half_size) {
    float distance_beyond_corner = min(((half_size.y - corner_radius) - abs(vertical_position)), 0.0);
    float curved_extent = ((half_size.x - corner_radius) + sqrt(max(0.0, ((corner_radius * corner_radius) - (distance_beyond_corner * distance_beyond_corner)))));
    vec2 integration_bounds = (vec2(horizontal_position) + vec2(-(curved_extent), curved_extent));
    vec2 normalized_bounds = (integration_bounds * (0.70710677 / standard_deviation_1));
    vec2 _e27 = approximate_error_function(normalized_bounds);
    vec2 integral = (vec2(0.5) + (0.5 * _e27));
    return (integral.y - integral.x);
}

float pick_corner_radius(vec2 center_to_point, Corners radii) {
    float left = ((center_to_point.y < 0.0) ? radii.top_left : radii.bottom_left);
    float right = ((center_to_point.y < 0.0) ? radii.top_right : radii.bottom_right);
    return ((center_to_point.x < 0.0) ? left : right);
}

float rounded_rectangle_signed_distance_from_corner(vec2 corner_center_to_point, float corner_radius_1) {
    if ((corner_radius_1 == 0.0)) {
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        return ((length(max(vec2(0.0, 0.0), corner_center_to_point)) + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y))) - corner_radius_1);
    }
}

float rounded_rectangle_signed_distance(vec2 point, Bounds bounds_5, Corners corner_radii) {
    vec2 _e3 = Bounds_1half_size(bounds_5);
    vec2 _e4 = Bounds_center(bounds_5);
    vec2 center_to_point_1 = (point - _e4);
    float _e6 = pick_corner_radius(center_to_point_1, corner_radii);
    float _e11 = rounded_rectangle_signed_distance_from_corner(((abs(center_to_point_1) - _e3) + vec2(_e6)), _e6);
    return _e11;
}

float gaussian_signed_distance_coverage(float distance_, float standard_deviation_2) {
    float normalized = (distance_ / (1.4142135 * standard_deviation_2));
    vec2 _e6 = approximate_error_function(vec2(normalized, normalized));
    return clamp((0.5 - (0.5 * _e6.x)), 0.0, 1.0);
}

Paint Paint_new(Background background, Bounds bounds_6) {
    return Paint(background, bounds_6);
}

PreparedPaint PreparedPaint_new(vec4 solid, vec4 color0_, vec4 color1_) {
    return PreparedPaint(solid, color0_, color1_);
}

PreparedPaint prepare_paint(Paint paint) {
    PreparedPaint prepared = PreparedPaint(vec4(0.0), vec4(0.0), vec4(0.0));
    vec4 _e1 = transparent();
    vec4 _e2 = transparent();
    vec4 _e3 = transparent();
    PreparedPaint _e4 = PreparedPaint_new(_e1, _e2, _e3);
    prepared = _e4;
    if ((paint.background.tag == BackgroundTag_LinearGradient)) {
        vec4 _e15 = hsla_to_rgba(paint.background.colors[0].color);
        prepared.color0_ = _e15;
        vec4 _e21 = hsla_to_rgba(paint.background.colors[1].color);
        prepared.color1_ = _e21;
        if ((paint.background.color_space == ColorSpace_Srgb)) {
            vec4 _e28 = prepared.color0_;
            vec4 _e29 = linear_to_srgba(_e28);
            prepared.color0_ = _e29;
            vec4 _e32 = prepared.color1_;
            vec4 _e33 = linear_to_srgba(_e32);
            prepared.color1_ = _e33;
        } else {
            vec4 _e36 = prepared.color0_;
            vec4 _e37 = linear_srgb_to_oklab(_e36);
            prepared.color0_ = _e37;
            vec4 _e40 = prepared.color1_;
            vec4 _e41 = linear_srgb_to_oklab(_e40);
            prepared.color1_ = _e41;
        }
    } else {
        vec4 _e45 = hsla_to_rgba(paint.background.solid);
        prepared.solid = _e45;
    }
    PreparedPaint _e46 = prepared;
    return _e46;
}

float linear_gradient_ratio(Paint paint_1, vec2 position_5) {
    vec2 direction = vec2(0.0);
    float ratio = 0.0;
    float radians_ = ((((paint_1.background.gradient_angle_or_pattern_height - FULL_TURN_DEGREES * trunc(paint_1.background.gradient_angle_or_pattern_height / FULL_TURN_DEGREES)) - CSS_GRADIENT_OFFSET_DEGREES) * PI) / HALF_TURN_DEGREES);
    direction = vec2(cos(radians_), sin(radians_));
    if ((paint_1.bounds.size.x > paint_1.bounds.size.y)) {
        float _e24 = direction.y;
        direction.y = (_e24 * (paint_1.bounds.size.y / paint_1.bounds.size.x));
    } else {
        float _e34 = direction.x;
        direction.x = (_e34 * (paint_1.bounds.size.x / paint_1.bounds.size.y));
    }
    vec2 _e44 = Bounds_1half_size(paint_1.bounds);
    vec2 _e46 = Bounds_center(paint_1.bounds);
    vec2 _e48 = direction;
    vec2 _e50 = direction;
    ratio = (dot((position_5 - _e46), _e48) / length(_e50));
    float _e55 = direction.x;
    float _e58 = direction.y;
    if ((abs(_e55) > abs(_e58))) {
        float _e61 = ratio;
        ratio = ((_e61 + _e44.x) / paint_1.bounds.size.x);
    } else {
        float _e68 = ratio;
        ratio = ((_e68 + _e44.y) / paint_1.bounds.size.y);
    }
    float first_stop = paint_1.background.colors[0].percentage;
    float last_stop = paint_1.background.colors[1].percentage;
    float _e83 = ratio;
    return clamp(((_e83 - first_stop) / (last_stop - first_stop)), 0.0, 1.0);
}

vec4 gradient_dither(vec2 position_6) {
    vec2 seed = (position_6 * GRADIENT_DITHER_SCALE);
    float noise_a = fract((sin(dot(seed, GRADIENT_DITHER_SEED_A)) * GRADIENT_DITHER_MULTIPLIER_A));
    float noise_b = fract((sin(dot(seed, GRADIENT_DITHER_SEED_B)) * GRADIENT_DITHER_MULTIPLIER_B));
    float triangular_noise = ((noise_a + noise_b) - 1.0);
    return vec4((triangular_noise * GRADIENT_DITHER_RGB_STRENGTH), (triangular_noise * GRADIENT_DITHER_RGB_STRENGTH), (triangular_noise * GRADIENT_DITHER_RGB_STRENGTH), (triangular_noise * GRADIENT_DITHER_ALPHA_STRENGTH));
}

vec4 linear_gradient_color(Paint paint_2, vec2 position_7, PreparedPaint prepared_1) {
    vec4 color_10 = vec4(0.0);
    float _e3 = linear_gradient_ratio(paint_2, position_7);
    vec4 ratio4_ = vec4(_e3, _e3, _e3, _e3);
    vec4 interpolated = mix(prepared_1.color0_, prepared_1.color1_, ratio4_);
    vec4 _e8 = transparent();
    color_10 = _e8;
    if ((paint_2.background.color_space == ColorSpace_Oklab)) {
        vec4 _e14 = oklab_to_linear_srgb(interpolated);
        color_10 = _e14;
    } else {
        vec4 _e15 = srgba_to_linear(interpolated);
        color_10 = _e15;
    }
    vec4 _e16 = color_10;
    vec4 _e17 = gradient_dither(position_7);
    return (_e16 + _e17);
}

vec4 slash_pattern_color(Paint paint_3, vec2 position_8, vec4 solid_1) {
    vec4 color_11 = vec4(0.0);
    float encoded = paint_3.background.gradient_angle_or_pattern_height;
    float pattern_width = ((encoded / PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE);
    float pattern_interval = ((encoded - PATTERN_PACKING_RADIX * trunc(encoded / PATTERN_PACKING_RADIX)) / PATTERN_COMPONENT_SCALE);
    float pattern_height = (pattern_width + pattern_interval);
    float period_1 = (pattern_height * 0.70710677);
    mat2x2 rotation = mat2x2(vec2(0.70710677, -0.70710677), vec2(0.70710677, 0.70710677));
    float pattern = ((rotation * (position_8 - paint_3.bounds.origin)).x - period_1 * trunc((rotation * (position_8 - paint_3.bounds.origin)).x / period_1));
    float distance_3 = (min(pattern, (period_1 - pattern)) - ((period_1 * (pattern_width / pattern_height)) / 2.0));
    color_11 = solid_1;
    float _e38 = color_11.w;
    float _e39 = antialiased_coverage(distance_3);
    color_11.w = (_e38 * _e39);
    vec4 _e41 = color_11;
    return _e41;
}

vec4 checkerboard_color(Paint paint_4, vec2 position_9, vec4 solid_2) {
    vec4 color_12 = vec4(0.0);
    vec2 relative = (position_9 - paint_4.bounds.origin);
    float square_size = paint_4.background.gradient_angle_or_pattern_height;
    float colored = ((floor((relative.x / square_size)) + floor((relative.y / square_size))) - 2.0 * trunc((floor((relative.x / square_size)) + floor((relative.y / square_size))) / 2.0));
    color_12 = solid_2;
    float _e19 = color_12.w;
    color_12.w = (_e19 * clamp(colored, 0.0, 1.0));
    vec4 _e22 = color_12;
    return _e22;
}

vec4 paint_color(Paint paint_5, vec2 position_10, PreparedPaint prepared_2) {
    vec4 color_13 = vec4(0.0);
    color_13 = prepared_2.solid;
    switch(paint_5.background.tag) {
        case 0u: {
            color_13 = prepared_2.solid;
            break;
        }
        case 1u: {
            vec4 _e8 = linear_gradient_color(paint_5, position_10, prepared_2);
            color_13 = _e8;
            break;
        }
        case 2u: {
            vec4 _e10 = slash_pattern_color(paint_5, position_10, prepared_2.solid);
            color_13 = _e10;
            break;
        }
        case 3u: {
            vec4 _e12 = checkerboard_color(paint_5, position_10, prepared_2.solid);
            color_13 = _e12;
            break;
        }
        default: {
            break;
        }
    }
    vec4 _e13 = color_13;
    return _e13;
}

float corner_dash_velocity(float first, float second) {
    if ((first == 0.0)) {
        return second;
    } else {
        if ((second == 0.0)) {
            return first;
        } else {
            return min(first, second);
        }
    }
}

float signed_modulo(float value_2, float modulus) {
    return (value_2 - (modulus * trunc((value_2 / modulus))));
}

float dash_coverage(float position_11, float period, float dash_length, float dash_velocity) {
    float half_period = (period / 2.0);
    float half_dash_length = (dash_length / 2.0);
    float _e10 = signed_modulo(((position_11 + half_period) - half_dash_length), period);
    float centered = (_e10 - half_period);
    float signed_distance_1 = (abs(centered) - half_dash_length);
    float _e15 = antialiased_coverage((signed_distance_1 / dash_velocity));
    return _e15;
}

float quarter_ellipse_signed_distance(vec2 point_1, vec2 radii_1) {
    return (((length((point_1 / radii_1)) - 1.0) * (radii_1.x + radii_1.y)) * -0.5);
}

vec4 corner_values(Corners corner_radii_1) {
    return vec4(corner_radii_1.top_left, corner_radii_1.top_right, corner_radii_1.bottom_right, corner_radii_1.bottom_left);
}

float normalized_superellipse_power(float corner_smoothing) {
    float smoothing_1 = clamp(corner_smoothing, 0.0, 1.0);
    float normalized_diagonal = (1.0 - (SUPERELLIPSE_DIAGONAL_INSET / (1.0 + smoothing_1)));
    return (-0.6931472 / log(normalized_diagonal));
}

vec4 normalized_superellipse_reaches(Corners corner_radii_2, float corner_smoothing_1) {
    vec4 _e2 = corner_values(corner_radii_2);
    return (max(_e2, vec4(0.0, 0.0, 0.0, 0.0)) * (1.0 + clamp(corner_smoothing_1, 0.0, 1.0)));
}

bool can_use_normalized_superellipse(vec2 size, Corners corner_radii_3, float corner_smoothing_2) {
    bool local_6 = false;
    bool local_7 = false;
    bool local_8 = false;
    bool local_9 = false;
    bool local_10 = false;
    bool local_11 = false;
    bool local_12 = false;
    bool local_13 = false;
    bool local_14 = false;
    bool local_15 = false;
    vec4 _e3 = corner_values(corner_radii_3);
    vec4 radii_4 = max(_e3, vec4(0.0, 0.0, 0.0, 0.0));
    vec4 _e10 = normalized_superellipse_reaches(corner_radii_3, corner_smoothing_2);
    if ((corner_smoothing_2 > 0.0)) {
        local_6 = (size.x > 0.0);
    } else {
        local_6 = false;
    }
    bool _e19 = local_6;
    if (_e19) {
        local_7 = (size.y > 0.0);
    } else {
        local_7 = false;
    }
    bool _e26 = local_7;
    if (_e26) {
        if (!((radii_4.x > 0.0))) {
            local_9 = (radii_4.y > 0.0);
        } else {
            local_9 = true;
        }
        bool _e39 = local_9;
        if (!(_e39)) {
            local_10 = (radii_4.z > 0.0);
        } else {
            local_10 = true;
        }
        bool _e47 = local_10;
        if (!(_e47)) {
            local_11 = (radii_4.w > 0.0);
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
        local_12 = ((_e10.x + _e10.y) <= size.x);
    } else {
        local_12 = false;
    }
    bool _e66 = local_12;
    if (_e66) {
        local_13 = ((_e10.y + _e10.z) <= size.y);
    } else {
        local_13 = false;
    }
    bool _e75 = local_13;
    if (_e75) {
        local_14 = ((_e10.z + _e10.w) <= size.x);
    } else {
        local_14 = false;
    }
    bool _e84 = local_14;
    if (_e84) {
        local_15 = ((_e10.w + _e10.x) <= size.y);
    } else {
        local_15 = false;
    }
    bool _e93 = local_15;
    return _e93;
}

float normalized_superellipse_signed_distance_from_corner(vec2 corner_to_point, float corner_radius_2, float corner_smoothing_3, float power) {
    bool local_16 = false;
    bool local_17 = false;
    float extent = (max(corner_radius_2, 0.0) * (1.0 + clamp(corner_smoothing_3, 0.0, 1.0)));
    vec2 corner_center_to_point_1 = (corner_to_point + vec2(extent));
    if (!((extent <= 0.0))) {
        local_16 = (corner_center_to_point_1.x <= 0.0);
    } else {
        local_16 = true;
    }
    bool _e23 = local_16;
    if (!(_e23)) {
        local_17 = (corner_center_to_point_1.y <= 0.0);
    } else {
        local_17 = true;
    }
    bool _e31 = local_17;
    if (_e31) {
        return max(corner_to_point.x, corner_to_point.y);
    }
    vec2 normalized_1 = (corner_center_to_point_1 / vec2(extent));
    vec2 powered = pow(normalized_1, vec2(power, power));
    float gradient = (power * length(pow(normalized_1, vec2((power - 1.0), (power - 1.0)))));
    return ((extent * ((powered.x + powered.y) - 1.0)) / max(gradient, FIGMA_EPSILON));
}

bool can_use_compact_corner_selection(vec2 size_1, vec4 reaches) {
    bool local_18 = false;
    bool local_19 = false;
    bool local_20 = false;
    float half_short_side = (0.5 * min(size_1.x, size_1.y));
    if ((reaches.x <= half_short_side)) {
        local_18 = (reaches.y <= half_short_side);
    } else {
        local_18 = false;
    }
    bool _e14 = local_18;
    if (_e14) {
        local_19 = (reaches.z <= half_short_side);
    } else {
        local_19 = false;
    }
    bool _e20 = local_19;
    if (_e20) {
        local_20 = (reaches.w <= half_short_side);
    } else {
        local_20 = false;
    }
    bool _e26 = local_20;
    return _e26;
}

float compact_normalized_superellipse_signed_distance(vec2 point_2, Bounds bounds_7, Corners corner_radii_4, float corner_smoothing_4, float power_1) {
    vec2 _e5 = Bounds_1half_size(bounds_7);
    vec2 _e6 = Bounds_center(bounds_7);
    vec2 center_to_point_2 = (point_2 - _e6);
    float _e8 = pick_corner_radius(center_to_point_2, corner_radii_4);
    vec2 corner_to_point_2 = (abs(center_to_point_2) - _e5);
    float _e11 = normalized_superellipse_signed_distance_from_corner(corner_to_point_2, _e8, corner_smoothing_4, power_1);
    return _e11;
}

float figma_signed_distance(vec2 delta, vec2 normal, float distance_1) {
    return ((dot(delta, normal) >= 0.0) ? distance_1 : -(distance_1));
}

SignedDistanceSample figma_nearest_straight_sample(vec2 point_3, vec2 size_2, vec4 horizontal_extents, vec4 vertical_extents) {
    vec2 nearest_delta = vec2(0.0);
    vec2 nearest_normal = vec2(0.0, -1.0);
    float nearest_distance_squared = 0.0;
    vec2 candidate_delta = vec2(0.0);
    float candidate_distance_squared = 0.0;
    nearest_delta = (point_3 - vec2(clamp(point_3.x, horizontal_extents.x, max(horizontal_extents.x, (size_2.x - horizontal_extents.y))), 0.0));
    vec2 _e20 = nearest_delta;
    vec2 _e21 = nearest_delta;
    nearest_distance_squared = dot(_e20, _e21);
    candidate_delta = (point_3 - vec2(size_2.x, clamp(point_3.y, vertical_extents.y, max(vertical_extents.y, (size_2.y - vertical_extents.z)))));
    vec2 _e36 = candidate_delta;
    vec2 _e37 = candidate_delta;
    candidate_distance_squared = dot(_e36, _e37);
    float _e40 = candidate_distance_squared;
    float _e41 = nearest_distance_squared;
    if ((_e40 < _e41)) {
        vec2 _e43 = candidate_delta;
        nearest_delta = _e43;
        nearest_normal = vec2(1.0, 0.0);
        float _e47 = candidate_distance_squared;
        nearest_distance_squared = _e47;
    }
    candidate_delta = (point_3 - vec2(clamp(point_3.x, horizontal_extents.w, max(horizontal_extents.w, (size_2.x - horizontal_extents.z))), size_2.y));
    vec2 _e59 = candidate_delta;
    vec2 _e60 = candidate_delta;
    candidate_distance_squared = dot(_e59, _e60);
    float _e62 = candidate_distance_squared;
    float _e63 = nearest_distance_squared;
    if ((_e62 < _e63)) {
        vec2 _e65 = candidate_delta;
        nearest_delta = _e65;
        nearest_normal = vec2(0.0, 1.0);
        float _e69 = candidate_distance_squared;
        nearest_distance_squared = _e69;
    }
    candidate_delta = (point_3 - vec2(0.0, clamp(point_3.y, vertical_extents.x, max(vertical_extents.x, (size_2.y - vertical_extents.w)))));
    vec2 _e81 = candidate_delta;
    vec2 _e82 = candidate_delta;
    candidate_distance_squared = dot(_e81, _e82);
    float _e84 = candidate_distance_squared;
    float _e85 = nearest_distance_squared;
    if ((_e84 < _e85)) {
        vec2 _e87 = candidate_delta;
        nearest_delta = _e87;
        nearest_normal = vec2(-1.0, 0.0);
        float _e91 = candidate_distance_squared;
        nearest_distance_squared = _e91;
    }
    vec2 _e92 = nearest_delta;
    vec2 _e93 = nearest_normal;
    float _e94 = nearest_distance_squared;
    float _e96 = figma_signed_distance(_e92, _e93, sqrt(_e94));
    vec2 _e97 = nearest_normal;
    return SignedDistanceSample(_e96, _e97, 0.0, FIGMA_SEGMENT_STRAIGHT);
}

bool figma_is_corner_candidate(vec2 point_4, vec2 size_3, float horizontal_extent, float vertical_extent, uint corner) {
    bool local_21 = false;
    bool local_22 = false;
    bool local_23 = false;
    bool local_24 = false;
    bool local_25 = false;
    if (!((horizontal_extent <= FIGMA_EPSILON))) {
        local_21 = (vertical_extent <= FIGMA_EPSILON);
    } else {
        local_21 = true;
    }
    bool _e13 = local_21;
    if (_e13) {
        return false;
    }
    switch(corner) {
        case 0u: {
            if ((point_4.x <= horizontal_extent)) {
                local_22 = (point_4.y <= vertical_extent);
            } else {
                local_22 = false;
            }
            bool _e22 = local_22;
            return _e22;
        }
        case 1u: {
            if (((size_3.x - point_4.x) <= horizontal_extent)) {
                local_23 = (point_4.y <= vertical_extent);
            } else {
                local_23 = false;
            }
            bool _e32 = local_23;
            return _e32;
        }
        case 2u: {
            if (((size_3.x - point_4.x) <= horizontal_extent)) {
                local_24 = ((size_3.y - point_4.y) <= vertical_extent);
            } else {
                local_24 = false;
            }
            bool _e44 = local_24;
            return _e44;
        }
        default: {
            if ((point_4.x <= horizontal_extent)) {
                local_25 = ((size_3.y - point_4.y) <= vertical_extent);
            } else {
                local_25 = false;
            }
            bool _e54 = local_25;
            return _e54;
        }
    }
}

vec2 figma_corner_to_point(vec2 point_5, vec2 size_4, uint corner_1) {
    switch(corner_1) {
        case 0u: {
            return -(point_5);
        }
        case 1u: {
            return vec2((point_5.x - size_4.x), -(point_5.y));
        }
        case 2u: {
            return (point_5 - size_4);
        }
        default: {
            return vec2(-(point_5.x), (point_5.y - size_4.y));
        }
    }
}

float reach_aware_normalized_superellipse_signed_distance(vec2 point_6, Bounds bounds_8, Corners corner_radii_5, float corner_smoothing_5, vec4 reaches_1, float power_2) {
    float distance_2 = 0.0;
    uint corner_2 = 0u;
    vec2 local_point = (point_6 - bounds_8.origin);
    vec4 _e8 = corner_values(corner_radii_5);
    SignedDistanceSample _e10 = figma_nearest_straight_sample(local_point, bounds_8.size, reaches_1, reaches_1);
    distance_2 = _e10.distance_;
    while(true) {
        uint _e15 = corner_2;
        if ((_e15 < 4u)) {
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
                vec2 _e27 = figma_corner_to_point(local_point, bounds_8.size, _e26);
                uint _e28 = corner_2;
                float _e30 = normalized_superellipse_signed_distance_from_corner(_e27, _e8[_e28], corner_smoothing_5, power_2);
                float _e32 = distance_2;
                if ((abs(_e30) <= abs(_e32))) {
                    distance_2 = _e30;
                }
            }
            uint _e35 = corner_2;
            corner_2 = (_e35 + 1u);
        }
    }
    float _e38 = distance_2;
    return _e38;
}

float normalized_superellipse_signed_distance(vec2 point_7, Bounds bounds_9, Corners corner_radii_6, float corner_smoothing_6, vec4 reaches_2, float power_3) {
    bool _e7 = can_use_compact_corner_selection(bounds_9.size, reaches_2);
    if (_e7) {
        float _e8 = compact_normalized_superellipse_signed_distance(point_7, bounds_9, corner_radii_6, corner_smoothing_6, power_3);
        return _e8;
    }
    float _e9 = reach_aware_normalized_superellipse_signed_distance(point_7, bounds_9, corner_radii_6, corner_smoothing_6, reaches_2, power_3);
    return _e9;
}

vec2 figma_split_side(float length_, float first_radius, float second_radius) {
    float total_radius = (first_radius + second_radius);
    if ((total_radius == 0.0)) {
        return vec2(0.0, 0.0);
    }
    float first_budget = ((length_ * first_radius) / total_radius);
    return vec2(first_budget, (length_ - first_budget));
}

FigmaCornerLayout figma_corner_layout(vec2 size_5, Corners corner_radii_7) {
    float radii_2[4] = float[4](0.0, 0.0, 0.0, 0.0);
    float budgets[4] = float[4](-1.0, -1.0, -1.0, -1.0);
    uint order[4] = uint[4](0u, 1u, 3u, 2u);
    uint sort_pass = 0u;
    uint index = 0u;
    uint rank = 0u;
    uint horizontal_neighbor = 0u;
    uint vertical_neighbor = 0u;
    float horizontal_budget = 0.0;
    bool local_26 = false;
    float vertical_budget = 0.0;
    bool local_27 = false;
    radii_2 = float[4](max(corner_radii_7.top_left, 0.0), max(corner_radii_7.top_right, 0.0), max(corner_radii_7.bottom_right, 0.0), max(corner_radii_7.bottom_left, 0.0));
    while(true) {
        uint _e30 = sort_pass;
        if ((_e30 < 3u)) {
        } else {
            break;
        }
        {
            index = 0u;
            while(true) {
                uint _e35 = index;
                uint _e37 = sort_pass;
                if ((_e35 < (3u - _e37))) {
                } else {
                    break;
                }
                {
                    uint _e40 = index;
                    uint _e43 = order[uint(_e40)];
                    float _e46 = radii_2[uint(_e43)];
                    uint _e47 = index;
                    uint _e52 = order[uint((_e47 + 1u))];
                    float _e55 = radii_2[uint(_e52)];
                    if ((_e46 < _e55)) {
                        uint _e57 = index;
                        uint swap = order[uint(_e57)];
                        uint _e61 = index;
                        uint _e64 = index;
                        uint _e69 = order[uint((_e64 + 1u))];
                        order[uint(_e61)] = _e69;
                        uint _e70 = index;
                        order[uint((_e70 + 1u))] = swap;
                    }
                    uint _e75 = index;
                    index = (_e75 + 1u);
                }
            }
            uint _e78 = sort_pass;
            sort_pass = (_e78 + 1u);
        }
    }
    while(true) {
        uint _e83 = rank;
        if ((_e83 < 4u)) {
        } else {
            break;
        }
        {
            uint _e86 = rank;
            uint corner_5 = order[uint(_e86)];
            float radius = radii_2[uint(corner_5)];
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
            float horizontal_radius = radii_2[uint(_e105)];
            horizontal_budget = 0.0;
            if (!((radius != 0.0))) {
                local_26 = (horizontal_radius != 0.0);
            } else {
                local_26 = true;
            }
            bool _e119 = local_26;
            if (_e119) {
                uint _e120 = horizontal_neighbor;
                float _e123 = budgets[uint(_e120)];
                if ((_e123 >= 0.0)) {
                    uint _e127 = horizontal_neighbor;
                    float _e130 = budgets[uint(_e127)];
                    horizontal_budget = (size_5.x - _e130);
                } else {
                    horizontal_budget = ((size_5.x * radius) / (radius + horizontal_radius));
                }
            }
            uint _e136 = vertical_neighbor;
            float vertical_radius = radii_2[uint(_e136)];
            vertical_budget = 0.0;
            if (!((radius != 0.0))) {
                local_27 = (vertical_radius != 0.0);
            } else {
                local_27 = true;
            }
            bool _e150 = local_27;
            if (_e150) {
                uint _e151 = vertical_neighbor;
                float _e154 = budgets[uint(_e151)];
                if ((_e154 >= 0.0)) {
                    uint _e158 = vertical_neighbor;
                    float _e161 = budgets[uint(_e158)];
                    vertical_budget = (size_5.y - _e161);
                } else {
                    vertical_budget = ((size_5.y * radius) / (radius + vertical_radius));
                }
            }
            float _e167 = horizontal_budget;
            float _e168 = vertical_budget;
            float budget = max(0.0, min(_e167, _e168));
            budgets[uint(corner_5)] = budget;
            radii_2[uint(corner_5)] = min(radius, budget);
            uint _e177 = rank;
            rank = (_e177 + 1u);
        }
    }
    float _e182 = radii_2[0];
    float _e184 = radii_2[1];
    vec2 _e185 = figma_split_side(size_5.x, _e182, _e184);
    float _e188 = radii_2[3];
    float _e190 = radii_2[2];
    vec2 _e191 = figma_split_side(size_5.x, _e188, _e190);
    float _e194 = radii_2[0];
    float _e196 = radii_2[3];
    vec2 _e197 = figma_split_side(size_5.y, _e194, _e196);
    float _e200 = radii_2[1];
    float _e202 = radii_2[2];
    vec2 _e203 = figma_split_side(size_5.y, _e200, _e202);
    return FigmaCornerLayout(vec4(_e185.x, _e185.y, _e191.y, _e191.x), vec4(_e197.x, _e203.x, _e203.y, _e197.y));
}

vec4 figma_smoothing_factors(float corner_smoothing_7) {
    float smoothing_2 = clamp(corner_smoothing_7, 0.0, 1.0);
    float arc_sweep = (1.5707964 * (1.0 - smoothing_2));
    float beta = (0.7853982 * smoothing_2);
    float join_handle_factor = tan((0.5 * beta));
    return vec4(smoothing_2, (sin((0.5 * arc_sweep)) * 1.4142135), (join_handle_factor * cos(beta)), (join_handle_factor * sin(beta)));
}

FigmaCornerParams figma_corner_params(float corner_radius_3, float horizontal_reach, float vertical_reach, vec4 smoothing_factors) {
    float safe_horizontal_reach = max(horizontal_reach, 0.0);
    float safe_vertical_reach = max(vertical_reach, 0.0);
    float radius_1 = min(max(corner_radius_3, 0.0), min(safe_horizontal_reach, safe_vertical_reach));
    float smoothing_3 = ((radius_1 > FIGMA_EPSILON) ? smoothing_factors.x : 0.0);
    float desired_reach = (radius_1 * (1.0 + smoothing_3));
    float arc_sweep_1 = (1.5707964 * (1.0 - smoothing_3));
    float arc_delta = (smoothing_factors.y * radius_1);
    float c_4 = (smoothing_factors.z * radius_1);
    float d_4 = (smoothing_factors.w * radius_1);
    float core_length = ((arc_delta + c_4) + d_4);
    float ideal_b = max(((desired_reach - core_length) / 3.0), 0.0);
    float horizontal_available = max((safe_horizontal_reach - core_length), 0.0);
    float vertical_available = max((safe_vertical_reach - core_length), 0.0);
    float shared_b = min(ideal_b, min((horizontal_available * 0.8333333), (vertical_available * 0.8333333)));
    return FigmaCornerParams(radius_1, smoothing_3, arc_sweep_1, c_4, d_4, FigmaAxisParams((horizontal_available - shared_b), shared_b, safe_horizontal_reach), FigmaAxisParams((vertical_available - shared_b), shared_b, safe_vertical_reach));
}

FigmaCornerExtents figma_corner_extents(Corners corner_radii_8, vec4 horizontal_budgets, vec4 vertical_budgets, float corner_smoothing_8) {
    vec4 zero = vec4(0.0, 0.0, 0.0, 0.0);
    vec4 horizontal = max(horizontal_budgets, zero);
    vec4 vertical = max(vertical_budgets, zero);
    vec4 _e11 = corner_values(corner_radii_8);
    vec4 radii_5 = min(max(_e11, zero), min(horizontal, vertical));
    vec4 reaches_3 = (radii_5 * (1.0 + clamp(corner_smoothing_8, 0.0, 1.0)));
    return FigmaCornerExtents(min(reaches_3, horizontal), min(reaches_3, vertical));
}

PreparedCorners prepare_corners(vec2 size_6, Corners radii_3, float smoothing, bool allow_superellipse) {
    PreparedCorners prepared_3 = PreparedCorners(vec4(0.0), vec4(0.0), vec4(0.0), 0.0);
    bool local_28 = false;
    vec4 _e4 = corner_values(radii_3);
    prepared_3 = PreparedCorners(_e4, _e4, vec4(0.0, 1.0, 0.0, 0.0), 0.0);
    if ((smoothing <= 0.0)) {
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
        vec4 _e21 = normalized_superellipse_reaches(radii_3, smoothing);
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
    vec4 _e36 = figma_smoothing_factors(smoothing);
    prepared_3.smoothing_factors = _e36;
    PreparedCorners _e37 = prepared_3;
    return _e37;
}

vec2 figma_cubic_point(FigmaAxisParams axis, float c, float d, float t) {
    float x1_ = (3.0 * axis.a);
    float x2_ = (3.0 * (axis.b - axis.a));
    float x3_ = ((axis.a - (2.0 * axis.b)) + c);
    float t2_ = (t * t);
    return vec2((t * (x1_ + (t * (x2_ + (t * x3_))))), ((d * t2_) * t));
}

vec2 figma_cubic_derivative(FigmaAxisParams axis_1, float c_1, float d_1, float t_1) {
    float x1_1 = (3.0 * axis_1.a);
    float x2_1 = (3.0 * (axis_1.b - axis_1.a));
    float x3_1 = ((axis_1.a - (2.0 * axis_1.b)) + c_1);
    float t2_1 = (t_1 * t_1);
    return vec2(((x1_1 + ((2.0 * x2_1) * t_1)) + ((3.0 * x3_1) * t2_1)), ((3.0 * d_1) * t2_1));
}

vec2 figma_cubic_second_derivative(FigmaAxisParams axis_2, float c_2, float d_2, float t_2) {
    float x2_2 = (3.0 * (axis_2.b - axis_2.a));
    float x3_2 = ((axis_2.a - (2.0 * axis_2.b)) + c_2);
    return vec2(((2.0 * x2_2) + ((6.0 * x3_2) * t_2)), ((6.0 * d_2) * t_2));
}

CubicClosestPoint closest_figma_cubic(vec2 point_8, FigmaAxisParams axis_3, float c_3, float d_3) {
    float t_3 = 0.0;
    uint iteration = 0u;
    float closest_t = 0.0;
    vec2 closest_point = vec2(0.0);
    float closest_distance = 0.0;
    float y_seed = pow(clamp((point_8.y / max(d_3, 1e-5)), 0.0, 1.0), 0.33333334);
    vec2 _e14 = figma_cubic_point(axis_3, c_3, d_3, 1.0);
    float chord_seed = clamp((dot(point_8, _e14) / max(dot(_e14, _e14), FIGMA_EPSILON)), 0.0, 1.0);
    vec2 _e23 = figma_cubic_point(axis_3, c_3, d_3, y_seed);
    vec2 y_delta = (_e23 - point_8);
    vec2 _e25 = figma_cubic_point(axis_3, c_3, d_3, chord_seed);
    vec2 chord_delta = (_e25 - point_8);
    t_3 = ((dot(chord_delta, chord_delta) < dot(y_delta, y_delta)) ? chord_seed : y_seed);
    while(true) {
        uint _e34 = iteration;
        if ((_e34 < 4u)) {
        } else {
            break;
        }
        {
            float _e37 = t_3;
            vec2 _e38 = figma_cubic_point(axis_3, c_3, d_3, _e37);
            float _e39 = t_3;
            vec2 _e40 = figma_cubic_derivative(axis_3, c_3, d_3, _e39);
            float _e41 = t_3;
            vec2 _e42 = figma_cubic_second_derivative(axis_3, c_3, d_3, _e41);
            vec2 delta_1 = (_e38 - point_8);
            float denominator = (dot(_e40, _e40) + dot(delta_1, _e42));
            if ((abs(denominator) > FIGMA_EPSILON)) {
                float _e50 = t_3;
                t_3 = clamp((_e50 - (dot(delta_1, _e40) / denominator)), 0.0, 1.0);
            }
            uint _e57 = iteration;
            iteration = (_e57 + 1u);
        }
    }
    float _e60 = t_3;
    closest_t = _e60;
    float _e62 = t_3;
    vec2 _e63 = figma_cubic_point(axis_3, c_3, d_3, _e62);
    closest_point = _e63;
    vec2 _e65 = closest_point;
    closest_distance = length((point_8 - _e65));
    vec2 start = vec2(0.0, 0.0);
    float start_distance = length((point_8 - start));
    float _e74 = closest_distance;
    if ((start_distance < _e74)) {
        closest_t = 0.0;
        closest_point = start;
        closest_distance = start_distance;
    }
    float end_distance = length((point_8 - _e14));
    float _e79 = closest_distance;
    if ((end_distance < _e79)) {
        closest_t = 1.0;
        closest_point = _e14;
        closest_distance = end_distance;
    }
    vec2 _e82 = closest_point;
    float _e83 = closest_t;
    vec2 _e84 = figma_cubic_derivative(axis_3, c_3, d_3, _e83);
    float _e85 = closest_distance;
    float _e86 = closest_t;
    return CubicClosestPoint(_e82, _e84, _e85, _e86);
}

float figma_cross_2d(vec2 a, vec2 b) {
    return ((a.x * b.y) - (a.y * b.x));
}

vec2 figma_unfold_normal(vec2 normal_1, bool mirrored) {
    return (mirrored ? vec2(-(normal_1.y), normal_1.x) : vec2(normal_1.x, -(normal_1.y)));
}

SignedDistanceSample figma_corner_signed_distance(vec2 corner_to_point_1, FigmaCornerParams params) {
    bool local_29 = false;
    bool local_30 = false;
    SignedDistanceSample sample_2 = SignedDistanceSample(0.0, vec2(0.0), 0.0, 0u);
    vec2 z = (corner_to_point_1 + vec2(params.horizontal.p, params.vertical.p));
    if (!((params.radius <= FIGMA_EPSILON))) {
        local_29 = (z.x <= 0.0);
    } else {
        local_29 = true;
    }
    bool _e18 = local_29;
    if (!(_e18)) {
        local_30 = (z.y <= 0.0);
    } else {
        local_30 = true;
    }
    bool _e26 = local_30;
    if (_e26) {
        return SignedDistanceSample(max(corner_to_point_1.x, corner_to_point_1.y), ((corner_to_point_1.x > corner_to_point_1.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0)), 0.0, FIGMA_SEGMENT_STRAIGHT);
    }
    vec2 horizontal_point = vec2(z.x, (params.vertical.p - z.y));
    vec2 vertical_point = vec2(z.y, (params.horizontal.p - z.x));
    vec2 circle_center = vec2((params.horizontal.p - params.radius), params.radius);
    vec2 join = vec2(((params.horizontal.a + params.horizontal.b) + params.c), params.d);
    vec2 start_direction = ((join - circle_center) / vec2(params.radius));
    vec2 to_point = (horizontal_point - circle_center);
    float to_point_length = length(to_point);
    vec2 point_direction = ((to_point_length > FIGMA_EPSILON) ? (to_point / vec2(max(to_point_length, FIGMA_EPSILON))) : start_direction);
    float _e83 = figma_cross_2d(start_direction, point_direction);
    float arc_angle = clamp(atan(_e83, dot(start_direction, point_direction)), 0.0, params.arc_sweep);
    float arc_sine = sin(arc_angle);
    float arc_cosine = cos(arc_angle);
    vec2 arc_normal = vec2(((arc_cosine * start_direction.x) - (arc_sine * start_direction.y)), ((arc_sine * start_direction.x) + (arc_cosine * start_direction.y)));
    vec2 arc_point = (circle_center + (params.radius * arc_normal));
    vec2 arc_delta_1 = (horizontal_point - arc_point);
    float arc_t = ((params.arc_sweep > FIGMA_EPSILON) ? (arc_angle / max(params.arc_sweep, FIGMA_EPSILON)) : 0.0);
    float _e116 = figma_signed_distance(arc_delta_1, arc_normal, length(arc_delta_1));
    vec2 _e118 = figma_unfold_normal(arc_normal, false);
    sample_2 = SignedDistanceSample(_e116, _e118, arc_t, FIGMA_SEGMENT_ARC);
    if ((params.smoothing > FIGMA_EPSILON)) {
        vec2 horizontal_bounds_delta = max(vec2(0.0, 0.0), max(-(horizontal_point), (horizontal_point - join)));
        float _e134 = sample_2.distance_;
        float _e136 = sample_2.distance_;
        if ((dot(horizontal_bounds_delta, horizontal_bounds_delta) <= (((_e134 * _e136) * 1.000001) + FIGMA_EPSILON))) {
            CubicClosestPoint _e146 = closest_figma_cubic(horizontal_point, params.horizontal, params.c, params.d);
            vec2 normal_3 = normalize(vec2(_e146.tangent.y, -(_e146.tangent.x)));
            float _e157 = figma_signed_distance((horizontal_point - _e146.point), normal_3, _e146.distance_);
            float _e160 = sample_2.distance_;
            if ((abs(_e157) <= abs(_e160))) {
                sample_2.distance_ = _e157;
                vec2 _e166 = figma_unfold_normal(normal_3, false);
                sample_2.normal = _e166;
                sample_2.path_t = _e146.path_t;
                sample_2.segment = FIGMA_SEGMENT_FIRST_CUBIC;
            }
        }
        vec2 vertical_join = vec2(((params.vertical.a + params.vertical.b) + params.c), params.d);
        vec2 vertical_bounds_delta = max(vec2(0.0, 0.0), max(-(vertical_point), (vertical_point - vertical_join)));
        float _e189 = sample_2.distance_;
        float _e191 = sample_2.distance_;
        if ((dot(vertical_bounds_delta, vertical_bounds_delta) <= (((_e189 * _e191) * 1.000001) + FIGMA_EPSILON))) {
            CubicClosestPoint _e201 = closest_figma_cubic(vertical_point, params.vertical, params.c, params.d);
            vec2 normal_4 = normalize(vec2(_e201.tangent.y, -(_e201.tangent.x)));
            float _e212 = figma_signed_distance((vertical_point - _e201.point), normal_4, _e201.distance_);
            float _e215 = sample_2.distance_;
            if ((abs(_e212) <= abs(_e215))) {
                sample_2.distance_ = _e212;
                vec2 _e221 = figma_unfold_normal(normal_4, true);
                sample_2.normal = _e221;
                sample_2.path_t = _e201.path_t;
                sample_2.segment = FIGMA_SEGMENT_SECOND_CUBIC;
            }
        }
    }
    SignedDistanceSample _e226 = sample_2;
    return _e226;
}

float figma_cubic_length(FigmaCornerParams params_1, FigmaAxisParams axis_4, float end_t) {
    bool local_31 = false;
    float center = (clamp(end_t, 0.0, 1.0) / 2.0);
    if (!((center <= 0.0))) {
        local_31 = (params_1.smoothing <= 0.0);
    } else {
        local_31 = true;
    }
    bool _e17 = local_31;
    if (_e17) {
        return 0.0;
    }
    float offset1_ = (center * 0.5384693);
    float offset2_ = (center * 0.90617985);
    vec2 _e25 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, center);
    float speed0_ = length(_e25);
    vec2 _e30 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center - offset1_));
    vec2 _e35 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center + offset1_));
    float speed1_ = (length(_e30) + length(_e35));
    vec2 _e41 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center - offset2_));
    vec2 _e46 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center + offset2_));
    float speed2_ = (length(_e41) + length(_e46));
    return (center * (((0.5688889 * speed0_) + (0.47862867 * speed1_)) + (0.23692688 * speed2_)));
}

float figma_corner_length(FigmaCornerParams params_2) {
    float _e3 = figma_cubic_length(params_2, params_2.horizontal, 1.0);
    float _e10 = figma_cubic_length(params_2, params_2.vertical, 1.0);
    return ((_e3 + (params_2.radius * params_2.arc_sweep)) + _e10);
}

float figma_corner_progress(FigmaCornerParams params_3, SignedDistanceSample sample_3, float total_length) {
    if ((sample_3.segment == FIGMA_SEGMENT_FIRST_CUBIC)) {
        float _e8 = figma_cubic_length(params_3, params_3.horizontal, sample_3.path_t);
        return _e8;
    }
    if ((sample_3.segment == FIGMA_SEGMENT_ARC)) {
        float _e14 = figma_cubic_length(params_3, params_3.horizontal, 1.0);
        return (_e14 + ((params_3.radius * params_3.arc_sweep) * sample_3.path_t));
    }
    if ((sample_3.segment == FIGMA_SEGMENT_SECOND_CUBIC)) {
        float _e26 = figma_cubic_length(params_3, params_3.vertical, sample_3.path_t);
        return (total_length - _e26);
    }
    return 0.0;
}

bool figma_has_corner_candidate(vec2 point_9, vec2 size_7, vec4 horizontal_extents_1, vec4 vertical_extents_1) {
    bool local_32 = false;
    bool local_33 = false;
    bool local_34 = false;
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

vec2 figma_orient_corner_normal(vec2 normal_2, uint corner_3) {
    switch(corner_3) {
        case 0u: {
            return -(normal_2);
        }
        case 1u: {
            return vec2(normal_2.x, -(normal_2.y));
        }
        case 2u: {
            return normal_2;
        }
        default: {
            return vec2(-(normal_2.x), normal_2.y);
        }
    }
}

FigmaRectangleSample figma_smooth_rectangle_sample(vec2 point_10, Bounds bounds_10, Corners corner_radii_9, vec4 horizontal_reaches, vec4 vertical_reaches, vec4 smoothing_factors_1) {
    FigmaRectangleSample result = FigmaRectangleSample(SignedDistanceSample(0.0, vec2(0.0), 0.0, 0u), 0u);
    uint corner_4 = 0u;
    SignedDistanceSample candidate = SignedDistanceSample(0.0, vec2(0.0), 0.0, 0u);
    vec2 local_point_1 = (point_10 - bounds_10.origin);
    vec4 _e8 = corner_values(corner_radii_9);
    SignedDistanceSample _e10 = figma_nearest_straight_sample(local_point_1, bounds_10.size, horizontal_reaches, vertical_reaches);
    result = FigmaRectangleSample(_e10, FIGMA_NO_CORNER);
    bool _e15 = figma_has_corner_candidate(local_point_1, bounds_10.size, horizontal_reaches, vertical_reaches);
    if (!(_e15)) {
        FigmaRectangleSample _e17 = result;
        return _e17;
    }
    while(true) {
        uint _e20 = corner_4;
        if ((_e20 < 4u)) {
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
                if ((_e36.radius > FIGMA_EPSILON)) {
                    uint _e41 = corner_4;
                    vec2 _e42 = figma_corner_to_point(local_point_1, bounds_10.size, _e41);
                    SignedDistanceSample _e43 = figma_corner_signed_distance(_e42, _e36);
                    candidate = _e43;
                    vec2 _e47 = candidate.normal;
                    uint _e48 = corner_4;
                    vec2 _e49 = figma_orient_corner_normal(_e47, _e48);
                    candidate.normal = _e49;
                    float _e51 = candidate.distance_;
                    float _e55 = result.signed_distance.distance_;
                    if ((abs(_e51) <= abs(_e55))) {
                        SignedDistanceSample _e59 = candidate;
                        result.signed_distance = _e59;
                        uint _e61 = corner_4;
                        result.corner = _e61;
                    }
                }
            }
            uint _e62 = corner_4;
            corner_4 = (_e62 + 1u);
        }
    }
    FigmaRectangleSample _e65 = result;
    return _e65;
}

float prepared_corner_signed_distance(vec2 point_11, Bounds bounds_11, Corners corner_radii_10, float corner_smoothing_9, PreparedCorners prepared_4) {
    if ((prepared_4.superellipse_power > 0.0)) {
        float _e10 = normalized_superellipse_signed_distance(point_11, bounds_11, corner_radii_10, corner_smoothing_9, prepared_4.horizontal_reaches, prepared_4.superellipse_power);
        return _e10;
    }
    if ((prepared_4.smoothing_factors.x <= 0.0)) {
        float _e15 = rounded_rectangle_signed_distance(point_11, bounds_11, corner_radii_10);
        return _e15;
    }
    FigmaRectangleSample _e19 = figma_smooth_rectangle_sample(point_11, bounds_11, corner_radii_10, prepared_4.horizontal_reaches, prepared_4.vertical_reaches, prepared_4.smoothing_factors);
    return _e19.signed_distance.distance_;
}

vec4 blur_composite_color(vec2 position_12, float coverage_1) {
    vec2 _e6 = _group_1_binding_0_fs.target_size;
    vec4 blurred = textureLod(_group_1_binding_1_fs, vec2((position_12 / _e6)), 0.0);
    float _e12 = _group_1_binding_0_fs.opacity;
    float factor = (coverage_1 * _e12);
    return vec4((blurred.x * factor), (blurred.y * factor), (blurred.z * factor), (blurred.w * factor));
}

vec4 sample_blur_texture(vec2 texture_coordinates) {
    vec4 _e4 = textureLod(_group_1_binding_1_fs, vec2(texture_coordinates), 0.0);
    return _e4;
}

vec4 gaussian_blur(vec2 texture_coordinates_1) {
    vec4 weighted_color = vec4(0.0);
    float weight_sum = 0.0;
    uint sample_index = 1u;
    BlurUniforms uniforms = _group_1_binding_0_fs;
    float _e5 = gaussian(0.0, uniforms.standard_deviation);
    vec4 _e6 = sample_blur_texture(texture_coordinates_1);
    weighted_color = (_e6 * _e5);
    weight_sum = _e5;
    while(true) {
        uint _e12 = sample_index;
        if ((_e12 <= uniforms.sample_count)) {
        } else {
            break;
        }
        {
            uint _e15 = sample_index;
            float distance_4 = (float(_e15) * uniforms.sample_step);
            float _e20 = gaussian(distance_4, uniforms.standard_deviation);
            vec2 coordinate_offset = (uniforms.direction * distance_4);
            vec4 _e24 = sample_blur_texture((texture_coordinates_1 - coordinate_offset));
            vec4 _e26 = sample_blur_texture((texture_coordinates_1 + coordinate_offset));
            vec4 symmetric_pair = (_e24 + _e26);
            vec4 _e28 = weighted_color;
            weighted_color = (_e28 + (symmetric_pair * _e20));
            float _e31 = weight_sum;
            weight_sum = (_e31 + (2.0 * _e20));
            uint _e35 = sample_index;
            sample_index = (_e35 + 1u);
        }
    }
    vec4 _e38 = weighted_color;
    float _e39 = weight_sum;
    return (_e38 / vec4(max(_e39, MIN_NORMALIZED_WEIGHT)));
}

void main() {
    BlurVarying input_ = BlurVarying(gl_FragCoord, _vs2fs_location0, _vs2fs_location3);
    uint _e3 = _group_1_binding_0_fs.downsample_mode;
    if ((_e3 == DownsampleMode_HalfResolution)) {
        vec2 destination = floor(input_.position.xy);
        vec2 _e14 = _group_1_binding_0_fs.target_size;
        vec2 _e18 = _group_1_binding_0_fs.source_size;
        vec2 _e24 = _group_1_binding_0_fs.source_size;
        vec2 source_coordinates = min(((destination + vec2(0.5)) / _e14), ((_e18 - vec2(0.5)) / _e24));
        vec4 _e30 = textureLod(_group_1_binding_1_fs, vec2(source_coordinates), 0.0);
        _fs2p_location0 = _e30;
        return;
    }
    vec4 _e35 = textureLod(_group_1_binding_1_fs, vec2(input_.texture_coordinates), 0.0);
    _fs2p_location0 = _e35;
    return;
}

