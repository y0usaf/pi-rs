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
struct PathRasterizationVertex {
    vec2 xy_position;
    vec2 curve_position;
    Background color;
    Bounds bounds;
};
struct PathRasterizationVarying {
    vec4 position;
    vec2 curve_position;
    uint vertex_id;
    vec4 clip_distances;
};
struct DataRange {
    uint base_texel;
    uint element_count;
    uint padding0_;
    uint padding1_;
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
const uint DATA_TEXTURE_WIDTH = 1024u;

layout(std140) uniform GlobalUniforms_block_0Vertex { GlobalUniforms _group_0_binding_0_vs; };

uniform usampler2D _group_1_binding_0_vs;

layout(std140) uniform DataRange_block_1Vertex { DataRange _group_1_binding_4_vs; };

smooth out vec2 _vs2fs_location0;
flat out uint _vs2fs_location1;
smooth out vec4 _vs2fs_location3;

float ContentMask_alpha(ContentMask content_mask, vec2 position) {
    float alpha = 1.0;
    Bounds bounds_7 = content_mask.bounds;
    Edges fade = content_mask.fade_out;
    if ((fade.left > 0.0)) {
        float _e9 = alpha;
        alpha = (_e9 * clamp(((position.x - bounds_7.origin.x) / fade.left), 0.0, 1.0));
    }
    if ((fade.right > 0.0)) {
        float _e21 = alpha;
        alpha = (_e21 * clamp((((bounds_7.origin.x + bounds_7.size.x) - position.x) / fade.right), 0.0, 1.0));
    }
    if ((fade.top > 0.0)) {
        float _e36 = alpha;
        alpha = (_e36 * clamp(((position.y - bounds_7.origin.y) / fade.top), 0.0, 1.0));
    }
    if ((fade.bottom > 0.0)) {
        float _e48 = alpha;
        alpha = (_e48 * clamp((((bounds_7.origin.y + bounds_7.size.y) - position.y) / fade.bottom), 0.0, 1.0));
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

vec2 rectangle_corner(uint vertex_id_1) {
    return vec2(float((vertex_id_1 & 1u)), (0.5 * float((vertex_id_1 & 2u))));
}

vec4 viewport_to_clip_position(vec2 position_2) {
    vec2 _e3 = _group_0_binding_0_vs.viewport_size;
    vec2 clip_position = (((position_2 / _e3) * vec2(2.0, -2.0)) + vec2(-1.0, 1.0));
    return vec4(clip_position.x, clip_position.y, 0.0, 1.0);
}

RectangleVertex rectangle_vertex(uint vertex_id_2, Bounds bounds_3) {
    vec2 _e2 = rectangle_corner(vertex_id_2);
    vec2 _e3 = Bounds_position(bounds_3, _e2);
    vec4 _e4 = viewport_to_clip_position(_e3);
    return RectangleVertex(_e2, _e3, _e4);
}

RectangleVertex transformed_rectangle_vertex(uint vertex_id_3, Bounds bounds_4, TransformationMatrix transform_1) {
    vec2 _e3 = rectangle_corner(vertex_id_3);
    vec2 _e4 = Bounds_position(bounds_4, _e3);
    vec2 _e5 = TransformationMatrix_1transform_position(transform_1, _e4);
    vec4 _e6 = viewport_to_clip_position(_e5);
    return RectangleVertex(_e3, _e5, _e6);
}

FullscreenVertex fullscreen_vertex(uint vertex_id_4) {
    vec2 texture_coordinates = vec2(float(((vertex_id_4 << 1u) & 2u)), float((vertex_id_4 & 2u)));
    return FullscreenVertex(texture_coordinates, vec4(((texture_coordinates.x * 2.0) - 1.0), (1.0 - (texture_coordinates.y * 2.0)), 0.0, 1.0));
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

vec4 blend_color(vec4 color_9, float alpha_factor) {
    float alpha_7 = (color_9.w * alpha_factor);
    uint _e6 = _group_0_binding_0_vs.premultiplied_alpha;
    bool _e7 = is_enabled(_e6);
    float multiplier = (_e7 ? alpha_7 : 1.0);
    return vec4((color_9.x * multiplier), (color_9.y * multiplier), (color_9.z * multiplier), alpha_7);
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
    float distance_1 = (min(pattern, (period_1 - pattern)) - ((period_1 * (pattern_width / pattern_height)) / 2.0));
    color_11 = solid_1;
    float _e38 = color_11.w;
    float _e39 = antialiased_coverage(distance_1);
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

uint dl_scene_word(usampler2D t, uint base, uint w) {
    uint components[4] = uint[4](0u, 0u, 0u, 0u);
    uint linear_1 = (base + w);
    uvec4 texel = texelFetch(t, ivec2(uvec2(((linear_1 / 4u) % DATA_TEXTURE_WIDTH), ((linear_1 / 4u) / DATA_TEXTURE_WIDTH))), 0);
    components = uint[4](texel.x, texel.y, texel.z, texel.w);
    uint _e24 = components[(linear_1 % 4u)];
    return _e24;
}

Hsla dl_load_Hsla_impl(usampler2D t_1, uint base_1) {
    uint _e3 = dl_scene_word(t_1, base_1, 0u);
    uint _e6 = dl_scene_word(t_1, base_1, 1u);
    uint _e9 = dl_scene_word(t_1, base_1, 2u);
    uint _e12 = dl_scene_word(t_1, base_1, 3u);
    return Hsla(uintBitsToFloat(_e3), uintBitsToFloat(_e6), uintBitsToFloat(_e9), uintBitsToFloat(_e12));
}

LinearColorStop dl_load_LinearColorStop_impl(usampler2D t_2, uint base_2) {
    Hsla _e4 = dl_load_Hsla_impl(t_2, (base_2 + 0u));
    uint _e6 = dl_scene_word(t_2, base_2, 4u);
    return LinearColorStop(_e4, uintBitsToFloat(_e6));
}

Background dl_load_Background_impl(usampler2D t_3, uint base_3) {
    uint _e3 = dl_scene_word(t_3, base_3, 0u);
    uint _e5 = dl_scene_word(t_3, base_3, 1u);
    Hsla _e8 = dl_load_Hsla_impl(t_3, (base_3 + 2u));
    uint _e10 = dl_scene_word(t_3, base_3, 6u);
    LinearColorStop _e14 = dl_load_LinearColorStop_impl(t_3, (base_3 + 7u));
    LinearColorStop _e17 = dl_load_LinearColorStop_impl(t_3, (base_3 + 12u));
    uint _e20 = dl_scene_word(t_3, base_3, 17u);
    return Background(_e3, _e5, _e8, uintBitsToFloat(_e10), LinearColorStop[2](_e14, _e17), _e20);
}

Bounds dl_load_Bounds_impl(usampler2D t_4, uint base_4) {
    uint _e3 = dl_scene_word(t_4, base_4, 0u);
    uint _e6 = dl_scene_word(t_4, base_4, 1u);
    uint _e10 = dl_scene_word(t_4, base_4, 2u);
    uint _e13 = dl_scene_word(t_4, base_4, 3u);
    return Bounds(vec2(uintBitsToFloat(_e3), uintBitsToFloat(_e6)), vec2(uintBitsToFloat(_e10), uintBitsToFloat(_e13)));
}

PathRasterizationVertex dl_load_PathRasterizationVertex_impl(usampler2D t_5, uint base_5) {
    uint _e3 = dl_scene_word(t_5, base_5, 0u);
    uint _e6 = dl_scene_word(t_5, base_5, 1u);
    uint _e10 = dl_scene_word(t_5, base_5, 2u);
    uint _e13 = dl_scene_word(t_5, base_5, 3u);
    Background _e18 = dl_load_Background_impl(t_5, (base_5 + 4u));
    Bounds _e21 = dl_load_Bounds_impl(t_5, (base_5 + 22u));
    return PathRasterizationVertex(vec2(uintBitsToFloat(_e3), uintBitsToFloat(_e6)), vec2(uintBitsToFloat(_e10), uintBitsToFloat(_e13)), _e18, _e21);
}

PathRasterizationVertex dl_load_PATH_VERTICES(uint i) {
    uint _e4 = _group_1_binding_4_vs.base_texel;
    PathRasterizationVertex _e10 = dl_load_PathRasterizationVertex_impl(_group_1_binding_0_vs, ((_e4 * 4u) + (i * 26u)));
    return _e10;
}

void main() {
    uint vertex_id = uint(gl_VertexID);
    PathRasterizationVertex _e1 = dl_load_PATH_VERTICES(vertex_id);
    vec4 _e3 = viewport_to_clip_position(_e1.xy_position);
    vec4 _e7 = clip_distances(_e1.xy_position, _e1.bounds);
    PathRasterizationVarying _tmp_return = PathRasterizationVarying(_e3, _e1.curve_position, vertex_id, _e7);
    gl_Position = _tmp_return.position;
    _vs2fs_location0 = _tmp_return.curve_position;
    _vs2fs_location1 = _tmp_return.vertex_id;
    _vs2fs_location3 = _tmp_return.clip_distances;
    gl_Position.yz = vec2(-gl_Position.y, gl_Position.z * 2.0 - gl_Position.w);
    return;
}

