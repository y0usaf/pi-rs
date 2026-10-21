struct NagaConstants {
    int first_vertex;
    int first_instance;
    uint other;
};
cbuffer DrawConstants : register(b3) { NagaConstants _NagaConstants; }

struct GlobalUniforms {
    float2 viewport_size;
    uint premultiplied_alpha;
    uint padding;
};

struct FontRasterizationUniforms {
    float4 gamma_ratios;
    float grayscale_enhanced_contrast;
    float subpixel_enhanced_contrast;
    uint uses_blue_green_red_subpixel_order;
    uint padding;
};

struct Bounds {
    float2 origin;
    float2 size;
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
    int2 origin;
    int2 size;
};

struct AtlasTile {
    AtlasTextureId texture_id;
    uint tile_id;
    uint padding;
    AtlasBounds bounds;
};

struct TransformationMatrix {
    float2 rotation_scale_0; float2 rotation_scale_1;
    float2 translation;
};

struct RectangleVertex {
    float2 unit_position;
    float2 viewport_position;
    float4 clip_position;
};

struct FullscreenVertex {
    float2 texture_coordinates;
    int _pad1_0;
    int _pad1_1;
    float4 clip_position;
};

struct Paint {
    Background background;
    Bounds bounds;
};

struct PreparedPaint {
    float4 solid;
    float4 color0_;
    float4 color1_;
};

struct MonochromeSprite {
    uint order;
    uint padding;
    Bounds bounds;
    ContentMask content_mask;
    Hsla color;
    AtlasTile tile;
    TransformationMatrix transformation;
};

struct MonochromeSpriteVarying {
    float4 position : SV_Position;
    float2 tile_position : LOC0;
    nointerpolation float4 color : LOC1;
    nointerpolation uint sprite_id : LOC2;
    float4 clip_distances : LOC3;
};

static const float PI = 3.1415927;
static const float HALF_TURN_DEGREES = 180.0;
static const float FULL_TURN_DEGREES = 360.0;
static const float CSS_GRADIENT_OFFSET_DEGREES = 90.0;
static const float PIXEL_ANTIALIAS_RADIUS = 0.5;
static const float GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS = 3.0;
static const float PATTERN_COMPONENT_SCALE = 255.0;
static const float PATTERN_PACKING_RADIX = 65535.0;
static const float MIN_NORMALIZED_WEIGHT = 1e-5;
static const float MIN_PATH_GRADIENT = 0.001;
static const float UNDERLINE_WAVE_FREQUENCY = 2.0;
static const float UNDERLINE_WAVE_HEIGHT_RATIO = 0.8;
static const float GRADIENT_DITHER_SCALE = 0.618034;
static const float GRADIENT_DITHER_RGB_STRENGTH = 0.007843138;
static const float GRADIENT_DITHER_ALPHA_STRENGTH = 0.011764706;
static const float2 GRADIENT_DITHER_SEED_A = float2(12.9898, 78.233);
static const float2 GRADIENT_DITHER_SEED_B = float2(39.346, 11.135);
static const float GRADIENT_DITHER_MULTIPLIER_A = 43758.547;
static const float GRADIENT_DITHER_MULTIPLIER_B = 24634.635;
static const float DIAGONAL_STRIPE_ANGLE = 0.7853982;
static const float3 REC_601_LUMA_WEIGHTS = float3(0.3, 0.59, 0.11);
static const float3 LINEAR_RGB_LUMA_WEIGHTS = float3(0.2126, 0.7152, 0.0722);
static const float SRGB_DECODE_CUTOFF = 0.04045;
static const float SRGB_ENCODE_CUTOFF = 0.0031308;
static const float SRGB_TRANSFER_OFFSET = 0.055;
static const float SRGB_TRANSFER_SCALE = 1.055;
static const float SRGB_DECODE_EXPONENT = 2.4;
static const float SRGB_LINEAR_SCALE = 12.92;
static const float ERROR_FUNCTION_LINEAR_COEFFICIENT = 0.278393;
static const float ERROR_FUNCTION_QUADRATIC_COEFFICIENT = 0.230389;
static const float ERROR_FUNCTION_CUBIC_COEFFICIENT = 0.000972;
static const float ERROR_FUNCTION_QUARTIC_COEFFICIENT = 0.078108;
static const float DARK_TEXT_CONTRAST_SCALE = 4.0;
static const float DARK_TEXT_BRIGHTNESS_CUTOFF = 0.75;
static const float3x3 LINEAR_SRGB_TO_CONE_RESPONSE = float3x3(float3(0.41222146, 0.2119035, 0.08830246), float3(0.53633255, 0.6806995, 0.28171885), float3(0.051445995, 0.10739696, 0.6299787));
static const float3x3 CONE_RESPONSE_TO_OKLAB = float3x3(float3(0.21045426, 1.9779985, 0.025904037), float3(0.7936178, -2.4285922, 0.78277177), float3(-0.004072047, 0.4505939, -0.80867577));
static const float3x3 OKLAB_TO_CONE_RESPONSE = float3x3(float3(1.0, 1.0, 1.0), float3(0.39633778, -0.105561346, -0.08948418), float3(0.21580376, -0.06385417, -1.2914855));
static const float3x3 CONE_RESPONSE_TO_LINEAR_SRGB = float3x3(float3(4.0767417, -1.268438, -0.0041960864), float3(-3.3077116, 2.6097574, -0.7034186), float3(0.23096994, -0.34131938, 1.7076147));
static const uint ShaderBool_Disabled = 0u;
static const uint ShaderBool_Enabled = 1u;
static const uint BackgroundTag_Solid = 0u;
static const uint BackgroundTag_LinearGradient = 1u;
static const uint BackgroundTag_PatternSlash = 2u;
static const uint BackgroundTag_Checkerboard = 3u;
static const uint ColorSpace_Srgb = 0u;
static const uint ColorSpace_Oklab = 1u;
static const uint BorderStyle_Solid = 0u;
static const uint BorderStyle_Dashed = 1u;
static const uint SurfaceColorFormat_Rgba = 0u;
static const uint SurfaceColorFormat_Yuv = 1u;
static const uint BlurCompositeClip_None = 0u;
static const uint BlurCompositeClip_RoundedBounds = 1u;
static const uint DownsampleMode_Copy = 0u;
static const uint DownsampleMode_HalfResolution = 1u;

cbuffer GLOBALS : register(b0) { GlobalUniforms GLOBALS; }
cbuffer FONT_RASTERIZATION : register(b1) { FontRasterizationUniforms FONT_RASTERIZATION; }
ByteAddressBuffer MONOCHROME_SPRITES : register(t2);
Texture2D<float4> MONOCHROME_TEXTURE : register(t3);
SamplerState MONOCHROME_SAMPLER : register(s4);

struct VertexOutput_vertex_monochrome_sprite {
    float2 tile_position : LOC0;
    nointerpolation float4 color_14 : LOC1;
    nointerpolation uint sprite_id : LOC2;
    float4 clip_distances_1 : LOC3;
    float4 position_12 : SV_Position;
};

struct FragmentInput_fragment_monochrome_sprite {
    float2 tile_position_1 : LOC0;
    nointerpolation float4 color_15 : LOC1;
    nointerpolation uint sprite_id_1 : LOC2;
    float4 clip_distances_2 : LOC3;
    float4 position_13 : SV_Position;
};

float ContentMask_alpha(ContentMask content_mask, float2 position)
{
    float alpha = 1.0;

    Bounds bounds_7 = content_mask.bounds;
    Edges fade = content_mask.fade_out;
    if ((fade.left > 0.0)) {
        float _e16 = alpha;
        alpha = (_e16 * saturate(((position.x - bounds_7.origin.x) / fade.left)));
    }
    if ((fade.right > 0.0)) {
        float _e31 = alpha;
        alpha = (_e31 * saturate((((bounds_7.origin.x + bounds_7.size.x) - position.x) / fade.right)));
    }
    if ((fade.top > 0.0)) {
        float _e43 = alpha;
        alpha = (_e43 * saturate(((position.y - bounds_7.origin.y) / fade.top)));
    }
    if ((fade.bottom > 0.0)) {
        float _e58 = alpha;
        alpha = (_e58 * saturate((((bounds_7.origin.y + bounds_7.size.y) - position.y) / fade.bottom)));
    }
    float _e60 = alpha;
    return _e60;
}

float2 Bounds_position(Bounds bounds, float2 unit_position)
{
    return (bounds.origin + (unit_position * bounds.size));
}

float2 Bounds_1half_size(Bounds bounds_1)
{
    return (bounds_1.size / (2.0).xx);
}

float2 Bounds_center(Bounds bounds_2)
{
    const float2 _e2 = Bounds_1half_size(bounds_2);
    return (bounds_2.origin + _e2);
}

bool Corners_1is_zero(Corners corners)
{
    return ((((corners.top_left == 0.0) && (corners.top_right == 0.0)) && (corners.bottom_right == 0.0)) && (corners.bottom_left == 0.0));
}

bool Edges_1is_zero(Edges edges)
{
    return ((((edges.top == 0.0) && (edges.right == 0.0)) && (edges.bottom == 0.0)) && (edges.left == 0.0));
}

float2x2 GetMatrotation_scaleOnTransformationMatrix(TransformationMatrix obj) {
    return float2x2(obj.rotation_scale_0, obj.rotation_scale_1);
}

void SetMatrotation_scaleOnTransformationMatrix(TransformationMatrix obj, float2x2 mat) {
    obj.rotation_scale_0 = mat[0];
    obj.rotation_scale_1 = mat[1];
}

void SetMatVecrotation_scaleOnTransformationMatrix(TransformationMatrix obj, float2 vec, uint mat_idx) {
    switch(mat_idx) {
    case 0: { obj.rotation_scale_0 = vec; break; }
    case 1: { obj.rotation_scale_1 = vec; break; }
    }
}

void SetMatScalarrotation_scaleOnTransformationMatrix(TransformationMatrix obj, float scalar, uint mat_idx, uint vec_idx) {
    switch(mat_idx) {
    case 0: { obj.rotation_scale_0[vec_idx] = scalar; break; }
    case 1: { obj.rotation_scale_1[vec_idx] = scalar; break; }
    }
}

float2 TransformationMatrix_1transform_position(TransformationMatrix transform, float2 position_1)
{
    return (mul(position_1, transpose(GetMatrotation_scaleOnTransformationMatrix(transform))) + transform.translation);
}

bool is_enabled(uint value)
{
    return (value == ShaderBool_Enabled);
}

float4 transparent()
{
    return float4(0.0, 0.0, 0.0, 0.0);
}

float antialiased_coverage(float signed_distance)
{
    return saturate((PIXEL_ANTIALIAS_RADIUS - signed_distance));
}

float4 premultiply(float4 color, float coverage)
{
    float alpha_5 = (color.w * coverage);
    return float4((color.x * alpha_5), (color.y * alpha_5), (color.z * alpha_5), alpha_5);
}

float2 atlas_texture_coordinates(float2 unit_position_1, AtlasTile tile, uint2 atlas_size)
{
    float2 tile_origin = float2(float(tile.bounds.origin.x), float(tile.bounds.origin.y));
    float2 tile_size = float2(float(tile.bounds.size.x), float(tile.bounds.size.y));
    float2 texture_size = float2(float(atlas_size.x), float(atlas_size.y));
    return ((tile_origin + (unit_position_1 * tile_size)) / texture_size);
}

float color_brightness(float3 color_1)
{
    return dot(color_1, REC_601_LUMA_WEIGHTS);
}

float light_on_dark_contrast(float enhanced_contrast, float3 color_2)
{
    const float _e4 = color_brightness(color_2);
    float darkness = saturate((DARK_TEXT_CONTRAST_SCALE * (DARK_TEXT_BRIGHTNESS_CUTOFF - _e4)));
    return (enhanced_contrast * darkness);
}

float _1enhance_contrast_f32_(float alpha_1, float contrast)
{
    return ((alpha_1 * (contrast + 1.0)) / ((alpha_1 * contrast) + 1.0));
}

float _2apply_alpha_correction_f32_(float alpha_2, float brightness, float4 gamma_ratios)
{
    float brightness_adjustment = ((brightness * gamma_ratios.x) + gamma_ratios.y);
    float correction = ((brightness_adjustment * alpha_2) + ((brightness * gamma_ratios.z) + gamma_ratios.w));
    return (alpha_2 + ((alpha_2 * (1.0 - alpha_2)) * correction));
}

float apply_contrast_and_gamma_correction(float sample_, float3 color_3, float enhanced_contrast_factor, float4 gamma_ratios_1)
{
    const float _e4 = light_on_dark_contrast(enhanced_contrast_factor, color_3);
    const float _e5 = _1enhance_contrast_f32_(sample_, _e4);
    const float _e6 = color_brightness(color_3);
    const float _e7 = _2apply_alpha_correction_f32_(_e5, _e6, gamma_ratios_1);
    return _e7;
}

float3 _1enhance_contrast_vec3f(float3 alpha_3, float contrast_1)
{
    return ((alpha_3 * (contrast_1 + 1.0)) / ((alpha_3 * contrast_1) + (1.0).xxx));
}

float3 _2apply_alpha_correction_vec3f(float3 alpha_4, float3 brightness_1, float4 gamma_ratios_2)
{
    float3 brightness_adjustment_1 = ((brightness_1 * gamma_ratios_2.x) + (gamma_ratios_2.y).xxx);
    float3 correction_1 = ((brightness_adjustment_1 * alpha_4) + ((brightness_1 * gamma_ratios_2.z) + (gamma_ratios_2.w).xxx));
    return (alpha_4 + ((alpha_4 * ((1.0).xxx - alpha_4)) * correction_1));
}

float3 apply_contrast_and_gamma_correction3_(float3 sample_1, float3 color_4, float enhanced_contrast_factor_1, float4 gamma_ratios_3)
{
    const float _e4 = light_on_dark_contrast(enhanced_contrast_factor_1, color_4);
    const float3 _e5 = _1enhance_contrast_vec3f(sample_1, _e4);
    const float3 _e6 = _2apply_alpha_correction_vec3f(_e5, color_4, gamma_ratios_3);
    return _e6;
}

float2 rectangle_corner(uint vertex_id_1)
{
    return float2(float((vertex_id_1 & 1u)), (0.5 * float((vertex_id_1 & 2u))));
}

float4 viewport_to_clip_position(float2 position_2)
{
    float2 _e3 = GLOBALS.viewport_size;
    float2 clip_position = (((position_2 / _e3) * float2(2.0, -2.0)) + float2(-1.0, 1.0));
    return float4(clip_position.x, clip_position.y, 0.0, 1.0);
}

RectangleVertex ConstructRectangleVertex(float2 arg0, float2 arg1, float4 arg2) {
    RectangleVertex ret = (RectangleVertex)0;
    ret.unit_position = arg0;
    ret.viewport_position = arg1;
    ret.clip_position = arg2;
    return ret;
}

RectangleVertex rectangle_vertex(uint vertex_id_2, Bounds bounds_3)
{
    const float2 _e2 = rectangle_corner(vertex_id_2);
    const float2 _e3 = Bounds_position(bounds_3, _e2);
    const float4 _e4 = viewport_to_clip_position(_e3);
    const RectangleVertex rectanglevertex = ConstructRectangleVertex(_e2, _e3, _e4);
    return rectanglevertex;
}

RectangleVertex transformed_rectangle_vertex(uint vertex_id_3, Bounds bounds_4, TransformationMatrix transform_1)
{
    const float2 _e3 = rectangle_corner(vertex_id_3);
    const float2 _e4 = Bounds_position(bounds_4, _e3);
    const float2 _e5 = TransformationMatrix_1transform_position(transform_1, _e4);
    const float4 _e6 = viewport_to_clip_position(_e5);
    const RectangleVertex rectanglevertex_1 = ConstructRectangleVertex(_e3, _e5, _e6);
    return rectanglevertex_1;
}

FullscreenVertex ConstructFullscreenVertex(float2 arg0, float4 arg1) {
    FullscreenVertex ret = (FullscreenVertex)0;
    ret.texture_coordinates = arg0;
    ret.clip_position = arg1;
    return ret;
}

FullscreenVertex fullscreen_vertex(uint vertex_id_4)
{
    float2 texture_coordinates = float2(float(((vertex_id_4 << 1u) & 2u)), float((vertex_id_4 & 2u)));
    const FullscreenVertex fullscreenvertex = ConstructFullscreenVertex(texture_coordinates, float4(((texture_coordinates.x * 2.0) - 1.0), (1.0 - (texture_coordinates.y * 2.0)), 0.0, 1.0));
    return fullscreenvertex;
}

float4 clip_distances(float2 position_3, Bounds clip_bounds)
{
    float2 distance_from_top_left = (position_3 - clip_bounds.origin);
    float2 distance_from_bottom_right = ((clip_bounds.origin + clip_bounds.size) - position_3);
    return float4(distance_from_top_left.x, distance_from_bottom_right.x, distance_from_top_left.y, distance_from_bottom_right.y);
}

bool is_clipped(float4 distances)
{
    return (min(min(distances.x, distances.y), min(distances.z, distances.w)) < 0.0);
}

float4 unclipped_distances()
{
    return float4(1.0, 1.0, 1.0, 1.0);
}

float3 srgb_to_linear(float3 srgb)
{
    bool3 cutoff = bool3((srgb.x < SRGB_DECODE_CUTOFF), (srgb.y < SRGB_DECODE_CUTOFF), (srgb.z < SRGB_DECODE_CUTOFF));
    float3 higher = pow(((srgb + (0.055).xxx) / (1.055).xxx), float3(2.4, 2.4, 2.4));
    return (cutoff ? (srgb / (12.92).xxx) : higher);
}

float3 linear_to_srgb(float3 linear_)
{
    bool3 cutoff_1 = bool3((linear_.x < SRGB_ENCODE_CUTOFF), (linear_.y < SRGB_ENCODE_CUTOFF), (linear_.z < SRGB_ENCODE_CUTOFF));
    float3 higher_1 = ((SRGB_TRANSFER_SCALE * pow(linear_, float3(0.41666666, 0.41666666, 0.41666666))) - (0.055).xxx);
    return (cutoff_1 ? (linear_ * SRGB_LINEAR_SCALE) : higher_1);
}

float4 linear_to_srgba(float4 color_5)
{
    const float3 _e2 = linear_to_srgb(color_5.xyz);
    return float4(_e2.x, _e2.y, _e2.z, color_5.w);
}

float4 srgba_to_linear(float4 color_6)
{
    const float3 _e2 = srgb_to_linear(color_6.xyz);
    return float4(_e2.x, _e2.y, _e2.z, color_6.w);
}

float3 hue_to_unit_rgb(float hue)
{
    float3 phases = frac(float3(hue, (hue + 0.6666667), (hue + 0.33333334)));
    return saturate((abs(((phases * 6.0) - (3.0).xxx)) - (1.0).xxx));
}

float4 hsla_to_rgba(Hsla hsla)
{
    float chroma = (hsla.s * (1.0 - abs(((2.0 * hsla.l) - 1.0))));
    const float3 _e12 = hue_to_unit_rgb(hsla.h);
    float3 red_green_blue = (((_e12 - (0.5).xxx) * chroma) + (hsla.l).xxx);
    return float4(red_green_blue.x, red_green_blue.y, red_green_blue.z, hsla.a);
}

float4 linear_srgb_to_oklab(float4 color_7)
{
    float3 linear_srgb = pow(color_7.xyz, float3(2.2, 2.2, 2.2));
    float3 cone_response = mul(linear_srgb, LINEAR_SRGB_TO_CONE_RESPONSE);
    float3 cube_root = float3(0.33333334, 0.33333334, 0.33333334);
    float3 oklab = mul(pow(cone_response, cube_root), CONE_RESPONSE_TO_OKLAB);
    return float4(oklab.x, oklab.y, oklab.z, color_7.w);
}

float4 oklab_to_linear_srgb(float4 color_8)
{
    float3 cone_root = mul(color_8.xyz, OKLAB_TO_CONE_RESPONSE);
    float3 cone_response_1 = ((cone_root * cone_root) * cone_root);
    float3 linear_srgb_1 = mul(cone_response_1, CONE_RESPONSE_TO_LINEAR_SRGB);
    float3 srgb_1 = pow(linear_srgb_1, float3(0.45454547, 0.45454547, 0.45454547));
    return float4(srgb_1.x, srgb_1.y, srgb_1.z, color_8.w);
}

float4 over(float4 below, float4 above)
{
    float alpha_6 = (above.w + (below.w * (1.0 - above.w)));
    if ((alpha_6 == 0.0)) {
        const float4 _e11 = transparent();
        return _e11;
    }
    float3 red_green_blue_1 = (((above.xyz * above.w) + ((below.xyz * below.w) * (1.0 - above.w))) / (alpha_6).xxx);
    return float4(red_green_blue_1.x, red_green_blue_1.y, red_green_blue_1.z, alpha_6);
}

float gaussian(float position_4, float standard_deviation)
{
    float variance = (standard_deviation * standard_deviation);
    return (exp((-((position_4 * position_4)) / (2.0 * variance))) / (2.5066283 * standard_deviation));
}

float2 approximate_error_function(float2 value_1)
{
    float2 signs = sign(value_1);
    float2 absolute_value = abs(value_1);
    float2 polynomial = ((1.0).xx + (absolute_value * ((0.278393).xx + (absolute_value * ((0.230389).xx + (absolute_value * ((0.000972).xx + (absolute_value * ERROR_FUNCTION_QUARTIC_COEFFICIENT))))))));
    float2 polynomial_squared = (polynomial * polynomial);
    return (signs - (signs / (polynomial_squared * polynomial_squared)));
}

float integrated_rounded_rectangle_coverage(float horizontal_position, float vertical_position, float standard_deviation_1, float corner_radius, float2 half_size)
{
    float distance_beyond_corner = min(((half_size.y - corner_radius) - abs(vertical_position)), 0.0);
    float curved_extent = ((half_size.x - corner_radius) + sqrt(max(0.0, ((corner_radius * corner_radius) - (distance_beyond_corner * distance_beyond_corner)))));
    float2 integration_bounds = ((horizontal_position).xx + float2(-(curved_extent), curved_extent));
    float2 normalized_bounds = (integration_bounds * (0.70710677 / standard_deviation_1));
    const float2 _e27 = approximate_error_function(normalized_bounds);
    float2 integral = ((0.5).xx + (0.5 * _e27));
    return (integral.y - integral.x);
}

float pick_corner_radius(float2 center_to_point, Corners radii)
{
    float left = ((center_to_point.y < 0.0) ? radii.top_left : radii.bottom_left);
    float right = ((center_to_point.y < 0.0) ? radii.top_right : radii.bottom_right);
    return ((center_to_point.x < 0.0) ? left : right);
}

float rounded_rectangle_signed_distance_from_corner(float2 corner_center_to_point, float corner_radius_1)
{
    if ((corner_radius_1 == 0.0)) {
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        return ((length(max(float2(0.0, 0.0), corner_center_to_point)) + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y))) - corner_radius_1);
    }
}

float rounded_rectangle_signed_distance(float2 point_, Bounds bounds_5, Corners corner_radii)
{
    const float2 _e3 = Bounds_1half_size(bounds_5);
    const float2 _e4 = Bounds_center(bounds_5);
    float2 center_to_point_1 = (point_ - _e4);
    const float _e6 = pick_corner_radius(center_to_point_1, corner_radii);
    const float _e11 = rounded_rectangle_signed_distance_from_corner(((abs(center_to_point_1) - _e3) + (_e6).xx), _e6);
    return _e11;
}

float gaussian_signed_distance_coverage(float distance_, float standard_deviation_2)
{
    float normalized = (distance_ / (1.4142135 * standard_deviation_2));
    const float2 _e6 = approximate_error_function(float2(normalized, normalized));
    return saturate((0.5 - (0.5 * _e6.x)));
}

float4 blend_color(float4 color_9, float alpha_factor)
{
    float alpha_7 = (color_9.w * alpha_factor);
    uint _e7 = GLOBALS.premultiplied_alpha;
    const bool _e8 = is_enabled(_e7);
    float multiplier = (_e8 ? alpha_7 : 1.0);
    return float4((color_9.x * multiplier), (color_9.y * multiplier), (color_9.z * multiplier), alpha_7);
}

Paint ConstructPaint(Background arg0, Bounds arg1) {
    Paint ret = (Paint)0;
    ret.background = arg0;
    ret.bounds = arg1;
    return ret;
}

Paint Paint_new(Background background, Bounds bounds_6)
{
    const Paint paint_6 = ConstructPaint(background, bounds_6);
    return paint_6;
}

PreparedPaint ConstructPreparedPaint(float4 arg0, float4 arg1, float4 arg2) {
    PreparedPaint ret = (PreparedPaint)0;
    ret.solid = arg0;
    ret.color0_ = arg1;
    ret.color1_ = arg2;
    return ret;
}

PreparedPaint PreparedPaint_new(float4 solid, float4 color0_, float4 color1_)
{
    const PreparedPaint preparedpaint = ConstructPreparedPaint(solid, color0_, color1_);
    return preparedpaint;
}

PreparedPaint prepare_paint(Paint paint)
{
    PreparedPaint prepared = (PreparedPaint)0;

    const float4 _e1 = transparent();
    const float4 _e2 = transparent();
    const float4 _e3 = transparent();
    const PreparedPaint _e4 = PreparedPaint_new(_e1, _e2, _e3);
    prepared = _e4;
    if ((paint.background.tag == BackgroundTag_LinearGradient)) {
        const float4 _e15 = hsla_to_rgba(paint.background.colors[0].color);
        prepared.color0_ = _e15;
        const float4 _e21 = hsla_to_rgba(paint.background.colors[1].color);
        prepared.color1_ = _e21;
        if ((paint.background.color_space == ColorSpace_Srgb)) {
            float4 _e28 = prepared.color0_;
            const float4 _e29 = linear_to_srgba(_e28);
            prepared.color0_ = _e29;
            float4 _e32 = prepared.color1_;
            const float4 _e33 = linear_to_srgba(_e32);
            prepared.color1_ = _e33;
        } else {
            float4 _e36 = prepared.color0_;
            const float4 _e37 = linear_srgb_to_oklab(_e36);
            prepared.color0_ = _e37;
            float4 _e40 = prepared.color1_;
            const float4 _e41 = linear_srgb_to_oklab(_e40);
            prepared.color1_ = _e41;
        }
    } else {
        const float4 _e45 = hsla_to_rgba(paint.background.solid);
        prepared.solid = _e45;
    }
    PreparedPaint _e46 = prepared;
    const PreparedPaint preparedpaint_1 = _e46;
    return preparedpaint_1;
}

float linear_gradient_ratio(Paint paint_1, float2 position_5)
{
    float2 direction = (float2)0;
    float ratio = (float)0;

    float radians_ = (((fmod(paint_1.background.gradient_angle_or_pattern_height, FULL_TURN_DEGREES) - CSS_GRADIENT_OFFSET_DEGREES) * PI) / HALF_TURN_DEGREES);
    direction = float2(cos(radians_), sin(radians_));
    if ((paint_1.bounds.size.x > paint_1.bounds.size.y)) {
        float _e31 = direction.y;
        direction.y = (_e31 * (paint_1.bounds.size.y / paint_1.bounds.size.x));
    } else {
        float _e41 = direction.x;
        direction.x = (_e41 * (paint_1.bounds.size.x / paint_1.bounds.size.y));
    }
    const float2 _e44 = Bounds_1half_size(paint_1.bounds);
    const float2 _e46 = Bounds_center(paint_1.bounds);
    float2 _e48 = direction;
    float2 _e50 = direction;
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
    return saturate(((_e83 - first_stop) / (last_stop - first_stop)));
}

float4 gradient_dither(float2 position_6)
{
    float2 seed = (position_6 * GRADIENT_DITHER_SCALE);
    float noise_a = frac((sin(dot(seed, GRADIENT_DITHER_SEED_A)) * GRADIENT_DITHER_MULTIPLIER_A));
    float noise_b = frac((sin(dot(seed, GRADIENT_DITHER_SEED_B)) * GRADIENT_DITHER_MULTIPLIER_B));
    float triangular_noise = ((noise_a + noise_b) - 1.0);
    return float4((triangular_noise * GRADIENT_DITHER_RGB_STRENGTH), (triangular_noise * GRADIENT_DITHER_RGB_STRENGTH), (triangular_noise * GRADIENT_DITHER_RGB_STRENGTH), (triangular_noise * GRADIENT_DITHER_ALPHA_STRENGTH));
}

float4 linear_gradient_color(Paint paint_2, float2 position_7, PreparedPaint prepared_1)
{
    float4 color_10 = (float4)0;

    const float _e3 = linear_gradient_ratio(paint_2, position_7);
    float4 ratio4_ = float4(_e3, _e3, _e3, _e3);
    float4 interpolated = lerp(prepared_1.color0_, prepared_1.color1_, ratio4_);
    const float4 _e8 = transparent();
    color_10 = _e8;
    if ((paint_2.background.color_space == ColorSpace_Oklab)) {
        const float4 _e14 = oklab_to_linear_srgb(interpolated);
        color_10 = _e14;
    } else {
        const float4 _e15 = srgba_to_linear(interpolated);
        color_10 = _e15;
    }
    float4 _e16 = color_10;
    const float4 _e17 = gradient_dither(position_7);
    return (_e16 + _e17);
}

float4 slash_pattern_color(Paint paint_3, float2 position_8, float4 solid_1)
{
    float4 color_11 = (float4)0;

    float encoded = paint_3.background.gradient_angle_or_pattern_height;
    float pattern_width = ((encoded / PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE);
    float pattern_interval = (fmod(encoded, PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE);
    float pattern_height = (pattern_width + pattern_interval);
    float period_1 = (pattern_height * 0.70710677);
    float2x2 rotation = float2x2(float2(0.70710677, -0.70710677), float2(0.70710677, 0.70710677));
    float pattern = fmod(mul((position_8 - paint_3.bounds.origin), rotation).x, period_1);
    float distance_1 = (min(pattern, (period_1 - pattern)) - ((period_1 * (pattern_width / pattern_height)) / 2.0));
    color_11 = solid_1;
    const float _e38 = antialiased_coverage(distance_1);
    float _e39 = color_11.w;
    color_11.w = (_e39 * _e38);
    float4 _e41 = color_11;
    return _e41;
}

float4 checkerboard_color(Paint paint_4, float2 position_9, float4 solid_2)
{
    float4 color_12 = (float4)0;

    float2 relative = (position_9 - paint_4.bounds.origin);
    float square_size = paint_4.background.gradient_angle_or_pattern_height;
    float colored = fmod((floor((relative.x / square_size)) + floor((relative.y / square_size))), 2.0);
    color_12 = solid_2;
    float _e20 = color_12.w;
    color_12.w = (_e20 * saturate(colored));
    float4 _e22 = color_12;
    return _e22;
}

float4 paint_color(Paint paint_5, float2 position_10, PreparedPaint prepared_2)
{
    float4 color_13 = (float4)0;

    color_13 = prepared_2.solid;
    switch(paint_5.background.tag) {
        case 0u: {
            color_13 = prepared_2.solid;
            break;
        }
        case 1u: {
            const float4 _e8 = linear_gradient_color(paint_5, position_10, prepared_2);
            color_13 = _e8;
            break;
        }
        case 2u: {
            const float4 _e10 = slash_pattern_color(paint_5, position_10, prepared_2.solid);
            color_13 = _e10;
            break;
        }
        case 3u: {
            const float4 _e12 = checkerboard_color(paint_5, position_10, prepared_2.solid);
            color_13 = _e12;
            break;
        }
        default: {
            break;
        }
    }
    float4 _e13 = color_13;
    return _e13;
}

float corner_dash_velocity(float first, float second)
{
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

float signed_modulo(float value_2, float modulus)
{
    return (value_2 - (modulus * trunc((value_2 / modulus))));
}

float dash_coverage(float position_11, float period, float dash_length, float dash_velocity)
{
    float half_period = (period / 2.0);
    float half_dash_length = (dash_length / 2.0);
    const float _e10 = signed_modulo(((position_11 + half_period) - half_dash_length), period);
    float centered = (_e10 - half_period);
    float signed_distance_1 = (abs(centered) - half_dash_length);
    const float _e15 = antialiased_coverage((signed_distance_1 / dash_velocity));
    return _e15;
}

float quarter_ellipse_signed_distance(float2 point_1, float2 radii_1)
{
    return (((length((point_1 / radii_1)) - 1.0) * (radii_1.x + radii_1.y)) * -0.5);
}

MonochromeSpriteVarying ConstructMonochromeSpriteVarying(float4 arg0, float2 arg1, float4 arg2, uint arg3, float4 arg4) {
    MonochromeSpriteVarying ret = (MonochromeSpriteVarying)0;
    ret.position = arg0;
    ret.tile_position = arg1;
    ret.color = arg2;
    ret.sprite_id = arg3;
    ret.clip_distances = arg4;
    return ret;
}

Bounds ConstructBounds(float2 arg0, float2 arg1) {
    Bounds ret = (Bounds)0;
    ret.origin = arg0;
    ret.size = arg1;
    return ret;
}

Edges ConstructEdges(float arg0, float arg1, float arg2, float arg3) {
    Edges ret = (Edges)0;
    ret.top = arg0;
    ret.right = arg1;
    ret.bottom = arg2;
    ret.left = arg3;
    return ret;
}

ContentMask ConstructContentMask(Bounds arg0, Edges arg1) {
    ContentMask ret = (ContentMask)0;
    ret.bounds = arg0;
    ret.fade_out = arg1;
    return ret;
}

Hsla ConstructHsla(float arg0, float arg1, float arg2, float arg3) {
    Hsla ret = (Hsla)0;
    ret.h = arg0;
    ret.s = arg1;
    ret.l = arg2;
    ret.a = arg3;
    return ret;
}

AtlasTextureId ConstructAtlasTextureId(uint arg0, uint arg1) {
    AtlasTextureId ret = (AtlasTextureId)0;
    ret.index = arg0;
    ret.kind = arg1;
    return ret;
}

AtlasBounds ConstructAtlasBounds(int2 arg0, int2 arg1) {
    AtlasBounds ret = (AtlasBounds)0;
    ret.origin = arg0;
    ret.size = arg1;
    return ret;
}

AtlasTile ConstructAtlasTile(AtlasTextureId arg0, uint arg1, uint arg2, AtlasBounds arg3) {
    AtlasTile ret = (AtlasTile)0;
    ret.texture_id = arg0;
    ret.tile_id = arg1;
    ret.padding = arg2;
    ret.bounds = arg3;
    return ret;
}

TransformationMatrix ConstructTransformationMatrix(float2x2 arg0, float2 arg1) {
    TransformationMatrix ret = (TransformationMatrix)0;
    ret.rotation_scale_0 = arg0[0];
    ret.rotation_scale_1 = arg0[1];
    ret.translation = arg1;
    return ret;
}

MonochromeSprite ConstructMonochromeSprite(uint arg0, uint arg1, Bounds arg2, ContentMask arg3, Hsla arg4, AtlasTile arg5, TransformationMatrix arg6) {
    MonochromeSprite ret = (MonochromeSprite)0;
    ret.order = arg0;
    ret.padding = arg1;
    ret.bounds = arg2;
    ret.content_mask = arg3;
    ret.color = arg4;
    ret.tile = arg5;
    ret.transformation = arg6;
    return ret;
}

uint2 NagaDimensions2D(Texture2D<float4> tex)
{
    uint4 ret;
    tex.GetDimensions(0, ret.x, ret.y, ret.z);
    return ret.xy;
}

VertexOutput_vertex_monochrome_sprite vertex_monochrome_sprite(uint vertex_id : SV_VertexID, uint instance_id : SV_InstanceID)
{
    MonochromeSprite sprite = ConstructMonochromeSprite(asuint(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+0)), asuint(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+4)), ConstructBounds(asfloat(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+8+0)), asfloat(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+8+8))), ConstructContentMask(ConstructBounds(asfloat(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+24+0+0)), asfloat(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+24+0+8))), ConstructEdges(asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+24+16+0)), asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+24+16+4)), asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+24+16+8)), asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+24+16+12)))), ConstructHsla(asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+56+0)), asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+56+4)), asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+56+8)), asfloat(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+56+12))), ConstructAtlasTile(ConstructAtlasTextureId(asuint(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+72+0+0)), asuint(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+72+0+4))), asuint(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+72+8)), asuint(MONOCHROME_SPRITES.Load(uint((_NagaConstants.first_instance + instance_id))*128+72+12)), ConstructAtlasBounds(asint(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+72+16+0)), asint(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+72+16+8)))), ConstructTransformationMatrix(float2x2(asfloat(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+104+0+0)), asfloat(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+104+0+8))), asfloat(MONOCHROME_SPRITES.Load2(uint((_NagaConstants.first_instance + instance_id))*128+104+16))));
    const RectangleVertex _e8 = transformed_rectangle_vertex((_NagaConstants.first_vertex + vertex_id), sprite.bounds, sprite.transformation);
    const float2 _e14 = atlas_texture_coordinates(_e8.unit_position, sprite.tile, NagaDimensions2D(MONOCHROME_TEXTURE));
    const float4 _e16 = hsla_to_rgba(sprite.color);
    const float4 _e20 = clip_distances(_e8.viewport_position, sprite.content_mask.bounds);
    const MonochromeSpriteVarying monochromespritevarying = ConstructMonochromeSpriteVarying(_e8.clip_position, _e14, _e16, (_NagaConstants.first_instance + instance_id), _e20);
    const VertexOutput_vertex_monochrome_sprite monochromespritevarying_1 = { monochromespritevarying.tile_position, monochromespritevarying.color, monochromespritevarying.sprite_id, monochromespritevarying.clip_distances, monochromespritevarying.position };
    return monochromespritevarying_1;
}

float4 fragment_monochrome_sprite(FragmentInput_fragment_monochrome_sprite fragmentinput_fragment_monochrome_sprite) : SV_Target0
{
    MonochromeSpriteVarying input = { fragmentinput_fragment_monochrome_sprite.position_13, fragmentinput_fragment_monochrome_sprite.tile_position_1, fragmentinput_fragment_monochrome_sprite.color_15, fragmentinput_fragment_monochrome_sprite.sprite_id_1, fragmentinput_fragment_monochrome_sprite.clip_distances_2 };
    const bool _e2 = is_clipped(input.clip_distances);
    if (_e2) {
        const float4 _e3 = transparent();
        return _e3;
    }
    MonochromeSprite sprite_1 = ConstructMonochromeSprite(asuint(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+0)), asuint(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+4)), ConstructBounds(asfloat(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+8+0)), asfloat(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+8+8))), ConstructContentMask(ConstructBounds(asfloat(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+24+0+0)), asfloat(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+24+0+8))), ConstructEdges(asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+24+16+0)), asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+24+16+4)), asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+24+16+8)), asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+24+16+12)))), ConstructHsla(asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+56+0)), asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+56+4)), asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+56+8)), asfloat(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+56+12))), ConstructAtlasTile(ConstructAtlasTextureId(asuint(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+72+0+0)), asuint(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+72+0+4))), asuint(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+72+8)), asuint(MONOCHROME_SPRITES.Load(uint(input.sprite_id)*128+72+12)), ConstructAtlasBounds(asint(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+72+16+0)), asint(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+72+16+8)))), ConstructTransformationMatrix(float2x2(asfloat(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+104+0+0)), asfloat(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+104+0+8))), asfloat(MONOCHROME_SPRITES.Load2(uint(input.sprite_id)*128+104+16))));
    float4 _e13 = MONOCHROME_TEXTURE.SampleLevel(MONOCHROME_SAMPLER, input.tile_position, 0.0);
    float sample_2 = _e13.x;
    float _e19 = FONT_RASTERIZATION.grayscale_enhanced_contrast;
    float4 _e22 = FONT_RASTERIZATION.gamma_ratios;
    const float _e23 = apply_contrast_and_gamma_correction(sample_2, input.color.xyz, _e19, _e22);
    const float _e28 = ContentMask_alpha(sprite_1.content_mask, input.position.xy);
    const float4 _e30 = blend_color(input.color, (_e23 * _e28));
    return _e30;
}
