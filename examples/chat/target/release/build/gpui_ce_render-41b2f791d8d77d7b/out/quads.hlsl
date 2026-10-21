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

struct SignedDistanceSample {
    float distance_;
    int _pad1_0;
    float2 normal;
    float path_t;
    uint segment;
};

struct FigmaCornerLayout {
    float4 horizontal_budgets;
    float4 vertical_budgets;
};

struct FigmaCornerExtents {
    float4 horizontal;
    float4 vertical;
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
    float2 point_;
    float2 tangent;
    float distance_;
    float path_t;
};

struct FigmaRectangleSample {
    SignedDistanceSample signed_distance;
    uint corner;
    int _end_pad_0;
};

struct PreparedCorners {
    float4 horizontal_reaches;
    float4 vertical_reaches;
    float4 smoothing_factors;
    float superellipse_power;
    int _end_pad_0;
    int _end_pad_1;
    int _end_pad_2;
};

struct Quad {
    uint order;
    uint border_style;
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

struct QuadGeometry {
    float2 point_;
    float2 center_to_point;
    float corner_radius;
    int _pad3_0;
    float2 corner_center_to_point;
    float2 reduced_border;
    float2 straight_border_inner_corner_to_point;
    bool near_rounded_corner;
    bool unrounded;
    int _end_pad_0;
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
    float4 position;
    PreparedPaint border;
    uint quad_id;
    int _pad3_0;
    int _pad3_1;
    int _pad3_2;
    float4 clip_distances;
    PreparedPaint fill;
};

struct QuadVarying {
    float4 position : SV_Position;
    nointerpolation float4 border_solid : LOC0;
    nointerpolation uint quad_id : LOC1;
    float4 clip_distances : LOC2;
    nointerpolation float4 fill_solid : LOC3;
    nointerpolation float4 fill_color0_ : LOC4;
    nointerpolation float4 fill_color1_ : LOC5;
    nointerpolation float4 border_color0_ : LOC6;
    nointerpolation float4 border_color1_ : LOC7;
};

struct SmoothedQuadVarying {
    float4 position : SV_Position;
    nointerpolation float4 border_solid : LOC0;
    nointerpolation uint quad_id : LOC1;
    float4 clip_distances : LOC2;
    nointerpolation float4 fill_solid : LOC3;
    nointerpolation float4 fill_color0_ : LOC4;
    nointerpolation float4 fill_color1_ : LOC5;
    nointerpolation float4 horizontal_corner_reaches : LOC6;
    nointerpolation float4 vertical_corner_reaches : LOC7;
    nointerpolation float4 corner_lengths : LOC8;
    nointerpolation float4 smoothing_factors : LOC9;
    nointerpolation float superellipse_power : LOC10;
    nointerpolation float4 border_color0_ : LOC11;
    nointerpolation float4 border_color1_ : LOC12;
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
static const float SUPERELLIPSE_DIAGONAL_INSET = 0.29289323;
static const uint FIGMA_SEGMENT_STRAIGHT = 0u;
static const uint FIGMA_SEGMENT_FIRST_CUBIC = 1u;
static const uint FIGMA_SEGMENT_ARC = 2u;
static const uint FIGMA_SEGMENT_SECOND_CUBIC = 3u;
static const uint FIGMA_NO_CORNER = 4u;
static const float FIGMA_EPSILON = 1e-6;
static const float DEFINITELY_OUTSIDE_INNER_BORDER = -1.0;

cbuffer GLOBALS : register(b0) { GlobalUniforms GLOBALS; }
cbuffer FONT_RASTERIZATION : register(b1) { FontRasterizationUniforms FONT_RASTERIZATION; }
ByteAddressBuffer QUADS : register(t2);

struct VertexOutput_vertex_quad {
    nointerpolation float4 border_solid : LOC0;
    nointerpolation uint quad_id : LOC1;
    float4 clip_distances_1 : LOC2;
    nointerpolation float4 fill_solid : LOC3;
    nointerpolation float4 fill_color0_ : LOC4;
    nointerpolation float4 fill_color1_ : LOC5;
    nointerpolation float4 border_color0_ : LOC6;
    nointerpolation float4 border_color1_ : LOC7;
    float4 position_13 : SV_Position;
};

struct FragmentInput_fragment_quad {
    nointerpolation float4 border_solid_1 : LOC0;
    nointerpolation uint quad_id_1 : LOC1;
    float4 clip_distances_2 : LOC2;
    nointerpolation float4 fill_solid_1 : LOC3;
    nointerpolation float4 fill_color0_1 : LOC4;
    nointerpolation float4 fill_color1_1 : LOC5;
    nointerpolation float4 border_color0_1 : LOC6;
    nointerpolation float4 border_color1_1 : LOC7;
    float4 position_14 : SV_Position;
};

struct VertexOutput_vertex_smoothed_quad {
    nointerpolation float4 border_solid_2 : LOC0;
    nointerpolation uint quad_id_2 : LOC1;
    float4 clip_distances_3 : LOC2;
    nointerpolation float4 fill_solid_2 : LOC3;
    nointerpolation float4 fill_color0_2 : LOC4;
    nointerpolation float4 fill_color1_2 : LOC5;
    nointerpolation float4 horizontal_corner_reaches : LOC6;
    nointerpolation float4 vertical_corner_reaches : LOC7;
    nointerpolation float4 corner_lengths_3 : LOC8;
    nointerpolation float4 smoothing_factors_2 : LOC9;
    nointerpolation float superellipse_power : LOC10;
    nointerpolation float4 border_color0_2 : LOC11;
    nointerpolation float4 border_color1_2 : LOC12;
    float4 position_15 : SV_Position;
};

struct FragmentInput_fragment_smoothed_quad {
    nointerpolation float4 border_solid_3 : LOC0;
    nointerpolation uint quad_id_3 : LOC1;
    float4 clip_distances_4 : LOC2;
    nointerpolation float4 fill_solid_3 : LOC3;
    nointerpolation float4 fill_color0_3 : LOC4;
    nointerpolation float4 fill_color1_3 : LOC5;
    nointerpolation float4 horizontal_corner_reaches_1 : LOC6;
    nointerpolation float4 vertical_corner_reaches_1 : LOC7;
    nointerpolation float4 corner_lengths_4 : LOC8;
    nointerpolation float4 smoothing_factors_3 : LOC9;
    nointerpolation float superellipse_power_1 : LOC10;
    nointerpolation float4 border_color0_3 : LOC11;
    nointerpolation float4 border_color1_3 : LOC12;
    float4 position_16 : SV_Position;
};

float ContentMask_alpha(ContentMask content_mask, float2 position)
{
    float alpha = 1.0;

    Bounds bounds_13 = content_mask.bounds;
    Edges fade = content_mask.fade_out;
    if ((fade.left > 0.0)) {
        float _e16 = alpha;
        alpha = (_e16 * saturate(((position.x - bounds_13.origin.x) / fade.left)));
    }
    if ((fade.right > 0.0)) {
        float _e31 = alpha;
        alpha = (_e31 * saturate((((bounds_13.origin.x + bounds_13.size.x) - position.x) / fade.right)));
    }
    if ((fade.top > 0.0)) {
        float _e43 = alpha;
        alpha = (_e43 * saturate(((position.y - bounds_13.origin.y) / fade.top)));
    }
    if ((fade.bottom > 0.0)) {
        float _e58 = alpha;
        alpha = (_e58 * saturate((((bounds_13.origin.y + bounds_13.size.y) - position.y) / fade.bottom)));
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

float4 premultiply(float4 color_2, float coverage)
{
    float alpha_5 = (color_2.w * coverage);
    return float4((color_2.x * alpha_5), (color_2.y * alpha_5), (color_2.z * alpha_5), alpha_5);
}

float2 atlas_texture_coordinates(float2 unit_position_1, AtlasTile tile, uint2 atlas_size)
{
    float2 tile_origin = float2(float(tile.bounds.origin.x), float(tile.bounds.origin.y));
    float2 tile_size = float2(float(tile.bounds.size.x), float(tile.bounds.size.y));
    float2 texture_size = float2(float(atlas_size.x), float(atlas_size.y));
    return ((tile_origin + (unit_position_1 * tile_size)) / texture_size);
}

float color_brightness(float3 color_3)
{
    return dot(color_3, REC_601_LUMA_WEIGHTS);
}

float light_on_dark_contrast(float enhanced_contrast, float3 color_4)
{
    const float _e4 = color_brightness(color_4);
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

float apply_contrast_and_gamma_correction(float sample_, float3 color_5, float enhanced_contrast_factor, float4 gamma_ratios_1)
{
    const float _e4 = light_on_dark_contrast(enhanced_contrast_factor, color_5);
    const float _e5 = _1enhance_contrast_f32_(sample_, _e4);
    const float _e6 = color_brightness(color_5);
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

float3 apply_contrast_and_gamma_correction3_(float3 sample_1, float3 color_6, float enhanced_contrast_factor_1, float4 gamma_ratios_3)
{
    const float _e4 = light_on_dark_contrast(enhanced_contrast_factor_1, color_6);
    const float3 _e5 = _1enhance_contrast_vec3f(sample_1, _e4);
    const float3 _e6 = _2apply_alpha_correction_vec3f(_e5, color_6, gamma_ratios_3);
    return _e6;
}

float2 rectangle_corner(uint vertex_id_2)
{
    return float2(float((vertex_id_2 & 1u)), (0.5 * float((vertex_id_2 & 2u))));
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

RectangleVertex rectangle_vertex(uint vertex_id_3, Bounds bounds_3)
{
    const float2 _e2 = rectangle_corner(vertex_id_3);
    const float2 _e3 = Bounds_position(bounds_3, _e2);
    const float4 _e4 = viewport_to_clip_position(_e3);
    const RectangleVertex rectanglevertex = ConstructRectangleVertex(_e2, _e3, _e4);
    return rectanglevertex;
}

RectangleVertex transformed_rectangle_vertex(uint vertex_id_4, Bounds bounds_4, TransformationMatrix transform_1)
{
    const float2 _e3 = rectangle_corner(vertex_id_4);
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

FullscreenVertex fullscreen_vertex(uint vertex_id_5)
{
    float2 texture_coordinates = float2(float(((vertex_id_5 << 1u) & 2u)), float((vertex_id_5 & 2u)));
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

float4 linear_to_srgba(float4 color_7)
{
    const float3 _e2 = linear_to_srgb(color_7.xyz);
    return float4(_e2.x, _e2.y, _e2.z, color_7.w);
}

float4 srgba_to_linear(float4 color_8)
{
    const float3 _e2 = srgb_to_linear(color_8.xyz);
    return float4(_e2.x, _e2.y, _e2.z, color_8.w);
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

float4 linear_srgb_to_oklab(float4 color_9)
{
    float3 linear_srgb = pow(color_9.xyz, float3(2.2, 2.2, 2.2));
    float3 cone_response = mul(linear_srgb, LINEAR_SRGB_TO_CONE_RESPONSE);
    float3 cube_root = float3(0.33333334, 0.33333334, 0.33333334);
    float3 oklab = mul(pow(cone_response, cube_root), CONE_RESPONSE_TO_OKLAB);
    return float4(oklab.x, oklab.y, oklab.z, color_9.w);
}

float4 oklab_to_linear_srgb(float4 color_10)
{
    float3 cone_root = mul(color_10.xyz, OKLAB_TO_CONE_RESPONSE);
    float3 cone_response_1 = ((cone_root * cone_root) * cone_root);
    float3 linear_srgb_1 = mul(cone_response_1, CONE_RESPONSE_TO_LINEAR_SRGB);
    float3 srgb_1 = pow(linear_srgb_1, float3(0.45454547, 0.45454547, 0.45454547));
    return float4(srgb_1.x, srgb_1.y, srgb_1.z, color_10.w);
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

float4 blend_color(float4 color_11, float alpha_factor)
{
    float alpha_7 = (color_11.w * alpha_factor);
    uint _e7 = GLOBALS.premultiplied_alpha;
    const bool _e8 = is_enabled(_e7);
    float multiplier = (_e8 ? alpha_7 : 1.0);
    return float4((color_11.x * multiplier), (color_11.y * multiplier), (color_11.z * multiplier), alpha_7);
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
    float4 color_12 = (float4)0;

    const float _e3 = linear_gradient_ratio(paint_2, position_7);
    float4 ratio4_ = float4(_e3, _e3, _e3, _e3);
    float4 interpolated = lerp(prepared_1.color0_, prepared_1.color1_, ratio4_);
    const float4 _e8 = transparent();
    color_12 = _e8;
    if ((paint_2.background.color_space == ColorSpace_Oklab)) {
        const float4 _e14 = oklab_to_linear_srgb(interpolated);
        color_12 = _e14;
    } else {
        const float4 _e15 = srgba_to_linear(interpolated);
        color_12 = _e15;
    }
    float4 _e16 = color_12;
    const float4 _e17 = gradient_dither(position_7);
    return (_e16 + _e17);
}

float4 slash_pattern_color(Paint paint_3, float2 position_8, float4 solid_1)
{
    float4 color_13 = (float4)0;

    float encoded = paint_3.background.gradient_angle_or_pattern_height;
    float pattern_width = ((encoded / PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE);
    float pattern_interval = (fmod(encoded, PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE);
    float pattern_height = (pattern_width + pattern_interval);
    float period_1 = (pattern_height * 0.70710677);
    float2x2 rotation = float2x2(float2(0.70710677, -0.70710677), float2(0.70710677, 0.70710677));
    float pattern = fmod(mul((position_8 - paint_3.bounds.origin), rotation).x, period_1);
    float distance_3 = (min(pattern, (period_1 - pattern)) - ((period_1 * (pattern_width / pattern_height)) / 2.0));
    color_13 = solid_1;
    const float _e38 = antialiased_coverage(distance_3);
    float _e39 = color_13.w;
    color_13.w = (_e39 * _e38);
    float4 _e41 = color_13;
    return _e41;
}

float4 checkerboard_color(Paint paint_4, float2 position_9, float4 solid_2)
{
    float4 color_14 = (float4)0;

    float2 relative = (position_9 - paint_4.bounds.origin);
    float square_size = paint_4.background.gradient_angle_or_pattern_height;
    float colored = fmod((floor((relative.x / square_size)) + floor((relative.y / square_size))), 2.0);
    color_14 = solid_2;
    float _e20 = color_14.w;
    color_14.w = (_e20 * saturate(colored));
    float4 _e22 = color_14;
    return _e22;
}

float4 paint_color(Paint paint_5, float2 position_10, PreparedPaint prepared_2)
{
    float4 color_15 = (float4)0;

    color_15 = prepared_2.solid;
    switch(paint_5.background.tag) {
        case 0u: {
            color_15 = prepared_2.solid;
            break;
        }
        case 1u: {
            const float4 _e8 = linear_gradient_color(paint_5, position_10, prepared_2);
            color_15 = _e8;
            break;
        }
        case 2u: {
            const float4 _e10 = slash_pattern_color(paint_5, position_10, prepared_2.solid);
            color_15 = _e10;
            break;
        }
        case 3u: {
            const float4 _e12 = checkerboard_color(paint_5, position_10, prepared_2.solid);
            color_15 = _e12;
            break;
        }
        default: {
            break;
        }
    }
    float4 _e13 = color_15;
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

float4 corner_values(Corners corner_radii_1)
{
    return float4(corner_radii_1.top_left, corner_radii_1.top_right, corner_radii_1.bottom_right, corner_radii_1.bottom_left);
}

float normalized_superellipse_power(float corner_smoothing)
{
    float smoothing_1 = clamp(corner_smoothing, 0.0, 1.0);
    float normalized_diagonal = (1.0 - (SUPERELLIPSE_DIAGONAL_INSET / (1.0 + smoothing_1)));
    return (-0.6931472 / log(normalized_diagonal));
}

float4 normalized_superellipse_reaches(Corners corner_radii_2, float corner_smoothing_1)
{
    const float4 _e2 = corner_values(corner_radii_2);
    return (max(_e2, float4(0.0, 0.0, 0.0, 0.0)) * (1.0 + clamp(corner_smoothing_1, 0.0, 1.0)));
}

bool can_use_normalized_superellipse(float2 size, Corners corner_radii_3, float corner_smoothing_2)
{
    const float4 _e3 = corner_values(corner_radii_3);
    float4 radii_6 = max(_e3, float4(0.0, 0.0, 0.0, 0.0));
    const float4 _e10 = normalized_superellipse_reaches(corner_radii_3, corner_smoothing_2);
    return ((((((((corner_smoothing_2 > 0.0) && (size.x > 0.0)) && (size.y > 0.0)) && ((((radii_6.x > 0.0) || (radii_6.y > 0.0)) || (radii_6.z > 0.0)) || (radii_6.w > 0.0))) && ((_e10.x + _e10.y) <= size.x)) && ((_e10.y + _e10.z) <= size.y)) && ((_e10.z + _e10.w) <= size.x)) && ((_e10.w + _e10.x) <= size.y));
}

float normalized_superellipse_signed_distance_from_corner(float2 corner_to_point, float corner_radius_2, float corner_smoothing_3, float power)
{
    float extent = (max(corner_radius_2, 0.0) * (1.0 + clamp(corner_smoothing_3, 0.0, 1.0)));
    float2 corner_center_to_point_1 = (corner_to_point + (extent).xx);
    if ((((extent <= 0.0) || (corner_center_to_point_1.x <= 0.0)) || (corner_center_to_point_1.y <= 0.0))) {
        return max(corner_to_point.x, corner_to_point.y);
    }
    float2 normalized_1 = (corner_center_to_point_1 / (extent).xx);
    float2 powered = pow(normalized_1, float2(power, power));
    float gradient = (power * length(pow(normalized_1, float2((power - 1.0), (power - 1.0)))));
    return ((extent * ((powered.x + powered.y) - 1.0)) / max(gradient, FIGMA_EPSILON));
}

bool can_use_compact_corner_selection(float2 size_1, float4 reaches)
{
    float half_short_side = (0.5 * min(size_1.x, size_1.y));
    return ((((reaches.x <= half_short_side) && (reaches.y <= half_short_side)) && (reaches.z <= half_short_side)) && (reaches.w <= half_short_side));
}

float compact_normalized_superellipse_signed_distance(float2 point_2, Bounds bounds_7, Corners corner_radii_4, float corner_smoothing_4, float power_1)
{
    const float2 _e5 = Bounds_1half_size(bounds_7);
    const float2 _e6 = Bounds_center(bounds_7);
    float2 center_to_point_2 = (point_2 - _e6);
    const float _e8 = pick_corner_radius(center_to_point_2, corner_radii_4);
    float2 corner_to_point_2 = (abs(center_to_point_2) - _e5);
    const float _e11 = normalized_superellipse_signed_distance_from_corner(corner_to_point_2, _e8, corner_smoothing_4, power_1);
    return _e11;
}

float figma_signed_distance(float2 delta, float2 normal, float distance_1)
{
    return ((dot(delta, normal) >= 0.0) ? distance_1 : -(distance_1));
}

SignedDistanceSample ConstructSignedDistanceSample(float arg0, float2 arg1, float arg2, uint arg3) {
    SignedDistanceSample ret = (SignedDistanceSample)0;
    ret.distance_ = arg0;
    ret.normal = arg1;
    ret.path_t = arg2;
    ret.segment = arg3;
    return ret;
}

SignedDistanceSample figma_nearest_straight_sample(float2 point_3, float2 size_2, float4 horizontal_extents, float4 vertical_extents)
{
    float2 nearest_delta = (float2)0;
    float2 nearest_normal = float2(0.0, -1.0);
    float nearest_distance_squared = (float)0;
    float2 candidate_delta = (float2)0;
    float candidate_distance_squared = (float)0;

    nearest_delta = (point_3 - float2(clamp(point_3.x, horizontal_extents.x, max(horizontal_extents.x, (size_2.x - horizontal_extents.y))), 0.0));
    float2 _e20 = nearest_delta;
    float2 _e21 = nearest_delta;
    nearest_distance_squared = dot(_e20, _e21);
    candidate_delta = (point_3 - float2(size_2.x, clamp(point_3.y, vertical_extents.y, max(vertical_extents.y, (size_2.y - vertical_extents.z)))));
    float2 _e36 = candidate_delta;
    float2 _e37 = candidate_delta;
    candidate_distance_squared = dot(_e36, _e37);
    float _e40 = candidate_distance_squared;
    float _e41 = nearest_distance_squared;
    if ((_e40 < _e41)) {
        float2 _e43 = candidate_delta;
        nearest_delta = _e43;
        nearest_normal = float2(1.0, 0.0);
        float _e47 = candidate_distance_squared;
        nearest_distance_squared = _e47;
    }
    candidate_delta = (point_3 - float2(clamp(point_3.x, horizontal_extents.w, max(horizontal_extents.w, (size_2.x - horizontal_extents.z))), size_2.y));
    float2 _e59 = candidate_delta;
    float2 _e60 = candidate_delta;
    candidate_distance_squared = dot(_e59, _e60);
    float _e62 = candidate_distance_squared;
    float _e63 = nearest_distance_squared;
    if ((_e62 < _e63)) {
        float2 _e65 = candidate_delta;
        nearest_delta = _e65;
        nearest_normal = float2(0.0, 1.0);
        float _e69 = candidate_distance_squared;
        nearest_distance_squared = _e69;
    }
    candidate_delta = (point_3 - float2(0.0, clamp(point_3.y, vertical_extents.x, max(vertical_extents.x, (size_2.y - vertical_extents.w)))));
    float2 _e81 = candidate_delta;
    float2 _e82 = candidate_delta;
    candidate_distance_squared = dot(_e81, _e82);
    float _e84 = candidate_distance_squared;
    float _e85 = nearest_distance_squared;
    if ((_e84 < _e85)) {
        float2 _e87 = candidate_delta;
        nearest_delta = _e87;
        nearest_normal = float2(-1.0, 0.0);
        float _e91 = candidate_distance_squared;
        nearest_distance_squared = _e91;
    }
    float2 _e92 = nearest_delta;
    float2 _e93 = nearest_normal;
    float _e94 = nearest_distance_squared;
    const float _e96 = figma_signed_distance(_e92, _e93, sqrt(_e94));
    float2 _e97 = nearest_normal;
    const SignedDistanceSample signeddistancesample = ConstructSignedDistanceSample(_e96, _e97, 0.0, FIGMA_SEGMENT_STRAIGHT);
    return signeddistancesample;
}

bool figma_is_corner_candidate(float2 point_4, float2 size_3, float horizontal_extent, float vertical_extent, uint corner)
{
    if (((horizontal_extent <= FIGMA_EPSILON) || (vertical_extent <= FIGMA_EPSILON))) {
        return false;
    }
    switch(corner) {
        case 0u: {
            return ((point_4.x <= horizontal_extent) && (point_4.y <= vertical_extent));
        }
        case 1u: {
            return (((size_3.x - point_4.x) <= horizontal_extent) && (point_4.y <= vertical_extent));
        }
        case 2u: {
            return (((size_3.x - point_4.x) <= horizontal_extent) && ((size_3.y - point_4.y) <= vertical_extent));
        }
        default: {
            return ((point_4.x <= horizontal_extent) && ((size_3.y - point_4.y) <= vertical_extent));
        }
    }
}

float2 figma_corner_to_point(float2 point_5, float2 size_4, uint corner_1)
{
    switch(corner_1) {
        case 0u: {
            return -(point_5);
        }
        case 1u: {
            return float2((point_5.x - size_4.x), -(point_5.y));
        }
        case 2u: {
            return (point_5 - size_4);
        }
        default: {
            return float2(-(point_5.x), (point_5.y - size_4.y));
        }
    }
}

float reach_aware_normalized_superellipse_signed_distance(float2 point_6, Bounds bounds_8, Corners corner_radii_5, float corner_smoothing_5, float4 reaches_1, float power_2)
{
    float distance_2 = (float)0;
    uint corner_2 = 0u;

    float2 local_point = (point_6 - bounds_8.origin);
    const float4 _e8 = corner_values(corner_radii_5);
    const SignedDistanceSample _e10 = figma_nearest_straight_sample(local_point, bounds_8.size, reaches_1, reaches_1);
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
            const bool _e24 = figma_is_corner_candidate(local_point, bounds_8.size, reaches_1[min(uint(_e19), 3u)], reaches_1[min(uint(_e21), 3u)], _e23);
            if (_e24) {
                uint _e26 = corner_2;
                const float2 _e27 = figma_corner_to_point(local_point, bounds_8.size, _e26);
                uint _e28 = corner_2;
                const float _e30 = normalized_superellipse_signed_distance_from_corner(_e27, _e8[min(uint(_e28), 3u)], corner_smoothing_5, power_2);
                float _e32 = distance_2;
                if ((abs(_e30) <= abs(_e32))) {
                    distance_2 = _e30;
                }
            }
            uint _e36 = corner_2;
            corner_2 = (_e36 + 1u);
        }
    }
    float _e38 = distance_2;
    return _e38;
}

float normalized_superellipse_signed_distance(float2 point_7, Bounds bounds_9, Corners corner_radii_6, float corner_smoothing_6, float4 reaches_2, float power_3)
{
    const bool _e7 = can_use_compact_corner_selection(bounds_9.size, reaches_2);
    if (_e7) {
        const float _e8 = compact_normalized_superellipse_signed_distance(point_7, bounds_9, corner_radii_6, corner_smoothing_6, power_3);
        return _e8;
    }
    const float _e9 = reach_aware_normalized_superellipse_signed_distance(point_7, bounds_9, corner_radii_6, corner_smoothing_6, reaches_2, power_3);
    return _e9;
}

float2 figma_split_side(float length_, float first_radius, float second_radius)
{
    float total_radius = (first_radius + second_radius);
    if ((total_radius == 0.0)) {
        return float2(0.0, 0.0);
    }
    float first_budget = ((length_ * first_radius) / total_radius);
    return float2(first_budget, (length_ - first_budget));
}

typedef float ret_Constructarray4_float_[4];
ret_Constructarray4_float_ Constructarray4_float_(float arg0, float arg1, float arg2, float arg3) {
    float ret[4] = { arg0, arg1, arg2, arg3 };
    return ret;
}

typedef uint ret_Constructarray4_uint_[4];
ret_Constructarray4_uint_ Constructarray4_uint_(uint arg0, uint arg1, uint arg2, uint arg3) {
    uint ret[4] = { arg0, arg1, arg2, arg3 };
    return ret;
}

FigmaCornerLayout ConstructFigmaCornerLayout(float4 arg0, float4 arg1) {
    FigmaCornerLayout ret = (FigmaCornerLayout)0;
    ret.horizontal_budgets = arg0;
    ret.vertical_budgets = arg1;
    return ret;
}

FigmaCornerLayout figma_corner_layout(float2 size_5, Corners corner_radii_7)
{
    float radii_2[4] = (float[4])0;
    float budgets[4] = Constructarray4_float_(-1.0, -1.0, -1.0, -1.0);
    uint order[4] = Constructarray4_uint_(0u, 1u, 3u, 2u);
    uint sort_pass = 0u;
    uint index = (uint)0;
    uint rank = 0u;
    uint horizontal_neighbor = (uint)0;
    uint vertical_neighbor = (uint)0;
    float horizontal_budget = (float)0;
    float vertical_budget = (float)0;

    radii_2 = Constructarray4_float_(max(corner_radii_7.top_left, 0.0), max(corner_radii_7.top_right, 0.0), max(corner_radii_7.bottom_right, 0.0), max(corner_radii_7.bottom_left, 0.0));
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
                    uint _e43 = order[min(uint(uint(_e40)), 3u)];
                    float _e46 = radii_2[min(uint(uint(_e43)), 3u)];
                    uint _e47 = index;
                    uint _e52 = order[min(uint(uint((_e47 + 1u))), 3u)];
                    float _e55 = radii_2[min(uint(uint(_e52)), 3u)];
                    if ((_e46 < _e55)) {
                        uint _e57 = index;
                        uint swap = order[min(uint(uint(_e57)), 3u)];
                        uint _e61 = index;
                        uint _e64 = index;
                        uint _e69 = order[min(uint(uint((_e64 + 1u))), 3u)];
                        order[min(uint(uint(_e61)), 3u)] = _e69;
                        uint _e70 = index;
                        order[min(uint(uint((_e70 + 1u))), 3u)] = swap;
                    }
                    uint _e76 = index;
                    index = (_e76 + 1u);
                }
            }
            uint _e79 = sort_pass;
            sort_pass = (_e79 + 1u);
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
            uint corner_5 = order[min(uint(uint(_e86)), 3u)];
            float radius = radii_2[min(uint(uint(corner_5)), 3u)];
            horizontal_neighbor = 0u;
            vertical_neighbor = 0u;
            bool should_continue = false;
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
            float horizontal_radius = radii_2[min(uint(uint(_e105)), 3u)];
            horizontal_budget = 0.0;
            if (((radius != 0.0) || (horizontal_radius != 0.0))) {
                uint _e116 = horizontal_neighbor;
                float _e119 = budgets[min(uint(uint(_e116)), 3u)];
                if ((_e119 >= 0.0)) {
                    uint _e123 = horizontal_neighbor;
                    float _e126 = budgets[min(uint(uint(_e123)), 3u)];
                    horizontal_budget = (size_5.x - _e126);
                } else {
                    horizontal_budget = ((size_5.x * radius) / (radius + horizontal_radius));
                }
            }
            uint _e132 = vertical_neighbor;
            float vertical_radius = radii_2[min(uint(uint(_e132)), 3u)];
            vertical_budget = 0.0;
            if (((radius != 0.0) || (vertical_radius != 0.0))) {
                uint _e143 = vertical_neighbor;
                float _e146 = budgets[min(uint(uint(_e143)), 3u)];
                if ((_e146 >= 0.0)) {
                    uint _e150 = vertical_neighbor;
                    float _e153 = budgets[min(uint(uint(_e150)), 3u)];
                    vertical_budget = (size_5.y - _e153);
                } else {
                    vertical_budget = ((size_5.y * radius) / (radius + vertical_radius));
                }
            }
            float _e160 = horizontal_budget;
            float _e161 = vertical_budget;
            float budget = max(0.0, min(_e160, _e161));
            budgets[min(uint(uint(corner_5)), 3u)] = budget;
            radii_2[min(uint(uint(corner_5)), 3u)] = min(radius, budget);
            uint _e170 = rank;
            rank = (_e170 + 1u);
        }
    }
    float _e174 = radii_2[0];
    float _e176 = radii_2[1];
    const float2 _e177 = figma_split_side(size_5.x, _e174, _e176);
    float _e180 = radii_2[3];
    float _e182 = radii_2[2];
    const float2 _e183 = figma_split_side(size_5.x, _e180, _e182);
    float _e186 = radii_2[0];
    float _e188 = radii_2[3];
    const float2 _e189 = figma_split_side(size_5.y, _e186, _e188);
    float _e192 = radii_2[1];
    float _e194 = radii_2[2];
    const float2 _e195 = figma_split_side(size_5.y, _e192, _e194);
    const FigmaCornerLayout figmacornerlayout = ConstructFigmaCornerLayout(float4(_e177.x, _e177.y, _e183.y, _e183.x), float4(_e189.x, _e195.x, _e195.y, _e189.y));
    return figmacornerlayout;
}

float4 figma_smoothing_factors(float corner_smoothing_7)
{
    float smoothing_2 = clamp(corner_smoothing_7, 0.0, 1.0);
    float arc_sweep = (1.5707964 * (1.0 - smoothing_2));
    float beta = (0.7853982 * smoothing_2);
    float join_handle_factor = tan((0.5 * beta));
    return float4(smoothing_2, (sin((0.5 * arc_sweep)) * 1.4142135), (join_handle_factor * cos(beta)), (join_handle_factor * sin(beta)));
}

FigmaAxisParams ConstructFigmaAxisParams(float arg0, float arg1, float arg2) {
    FigmaAxisParams ret = (FigmaAxisParams)0;
    ret.a = arg0;
    ret.b = arg1;
    ret.p = arg2;
    return ret;
}

FigmaCornerParams ConstructFigmaCornerParams(float arg0, float arg1, float arg2, float arg3, float arg4, FigmaAxisParams arg5, FigmaAxisParams arg6) {
    FigmaCornerParams ret = (FigmaCornerParams)0;
    ret.radius = arg0;
    ret.smoothing = arg1;
    ret.arc_sweep = arg2;
    ret.c = arg3;
    ret.d = arg4;
    ret.horizontal = arg5;
    ret.vertical = arg6;
    return ret;
}

FigmaCornerParams figma_corner_params(float corner_radius_3, float horizontal_reach, float vertical_reach, float4 smoothing_factors)
{
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
    const FigmaCornerParams figmacornerparams = ConstructFigmaCornerParams(radius_1, smoothing_3, arc_sweep_1, c_4, d_4, ConstructFigmaAxisParams((horizontal_available - shared_b), shared_b, safe_horizontal_reach), ConstructFigmaAxisParams((vertical_available - shared_b), shared_b, safe_vertical_reach));
    return figmacornerparams;
}

FigmaCornerExtents ConstructFigmaCornerExtents(float4 arg0, float4 arg1) {
    FigmaCornerExtents ret = (FigmaCornerExtents)0;
    ret.horizontal = arg0;
    ret.vertical = arg1;
    return ret;
}

FigmaCornerExtents figma_corner_extents(Corners corner_radii_8, float4 horizontal_budgets, float4 vertical_budgets, float corner_smoothing_8)
{
    float4 zero = float4(0.0, 0.0, 0.0, 0.0);
    float4 horizontal_1 = max(horizontal_budgets, zero);
    float4 vertical = max(vertical_budgets, zero);
    const float4 _e11 = corner_values(corner_radii_8);
    float4 radii_7 = min(max(_e11, zero), min(horizontal_1, vertical));
    float4 reaches_3 = (radii_7 * (1.0 + clamp(corner_smoothing_8, 0.0, 1.0)));
    const FigmaCornerExtents figmacornerextents = ConstructFigmaCornerExtents(min(reaches_3, horizontal_1), min(reaches_3, vertical));
    return figmacornerextents;
}

PreparedCorners ConstructPreparedCorners(float4 arg0, float4 arg1, float4 arg2, float arg3) {
    PreparedCorners ret = (PreparedCorners)0;
    ret.horizontal_reaches = arg0;
    ret.vertical_reaches = arg1;
    ret.smoothing_factors = arg2;
    ret.superellipse_power = arg3;
    return ret;
}

PreparedCorners prepare_corners(float2 size_6, Corners radii_3, float smoothing, bool allow_superellipse)
{
    PreparedCorners prepared_3 = (PreparedCorners)0;

    const float4 _e4 = corner_values(radii_3);
    prepared_3 = ConstructPreparedCorners(_e4, _e4, float4(0.0, 1.0, 0.0, 0.0), 0.0);
    if ((smoothing <= 0.0)) {
        PreparedCorners _e15 = prepared_3;
        const PreparedCorners preparedcorners = _e15;
        return preparedcorners;
    }
    const bool _e16 = can_use_normalized_superellipse(size_6, radii_3, smoothing);
    if ((allow_superellipse && _e16)) {
        const float4 _e18 = normalized_superellipse_reaches(radii_3, smoothing);
        prepared_3.horizontal_reaches = _e18;
        prepared_3.vertical_reaches = _e18;
        const float _e22 = normalized_superellipse_power(smoothing);
        prepared_3.superellipse_power = _e22;
        PreparedCorners _e23 = prepared_3;
        const PreparedCorners preparedcorners_1 = _e23;
        return preparedcorners_1;
    }
    const FigmaCornerLayout _e24 = figma_corner_layout(size_6, radii_3);
    const FigmaCornerExtents _e27 = figma_corner_extents(radii_3, _e24.horizontal_budgets, _e24.vertical_budgets, smoothing);
    prepared_3.horizontal_reaches = _e27.horizontal;
    prepared_3.vertical_reaches = _e27.vertical;
    const float4 _e33 = figma_smoothing_factors(smoothing);
    prepared_3.smoothing_factors = _e33;
    PreparedCorners _e34 = prepared_3;
    const PreparedCorners preparedcorners_2 = _e34;
    return preparedcorners_2;
}

float2 figma_cubic_point(FigmaAxisParams axis, float c, float d, float t)
{
    float x1_ = (3.0 * axis.a);
    float x2_ = (3.0 * (axis.b - axis.a));
    float x3_ = ((axis.a - (2.0 * axis.b)) + c);
    float t2_ = (t * t);
    return float2((t * (x1_ + (t * (x2_ + (t * x3_))))), ((d * t2_) * t));
}

float2 figma_cubic_derivative(FigmaAxisParams axis_1, float c_1, float d_1, float t_1)
{
    float x1_1 = (3.0 * axis_1.a);
    float x2_1 = (3.0 * (axis_1.b - axis_1.a));
    float x3_1 = ((axis_1.a - (2.0 * axis_1.b)) + c_1);
    float t2_1 = (t_1 * t_1);
    return float2(((x1_1 + ((2.0 * x2_1) * t_1)) + ((3.0 * x3_1) * t2_1)), ((3.0 * d_1) * t2_1));
}

float2 figma_cubic_second_derivative(FigmaAxisParams axis_2, float c_2, float d_2, float t_2)
{
    float x2_2 = (3.0 * (axis_2.b - axis_2.a));
    float x3_2 = ((axis_2.a - (2.0 * axis_2.b)) + c_2);
    return float2(((2.0 * x2_2) + ((6.0 * x3_2) * t_2)), ((6.0 * d_2) * t_2));
}

CubicClosestPoint ConstructCubicClosestPoint(float2 arg0, float2 arg1, float arg2, float arg3) {
    CubicClosestPoint ret = (CubicClosestPoint)0;
    ret.point_ = arg0;
    ret.tangent = arg1;
    ret.distance_ = arg2;
    ret.path_t = arg3;
    return ret;
}

CubicClosestPoint closest_figma_cubic(float2 point_8, FigmaAxisParams axis_3, float c_3, float d_3)
{
    float t_3 = (float)0;
    uint iteration = 0u;
    float closest_t = (float)0;
    float2 closest_point = (float2)0;
    float closest_distance = (float)0;

    float y_seed = pow(clamp((point_8.y / max(d_3, 1e-5)), 0.0, 1.0), 0.33333334);
    const float2 _e14 = figma_cubic_point(axis_3, c_3, d_3, 1.0);
    float chord_seed = clamp((dot(point_8, _e14) / max(dot(_e14, _e14), FIGMA_EPSILON)), 0.0, 1.0);
    const float2 _e23 = figma_cubic_point(axis_3, c_3, d_3, y_seed);
    float2 y_delta = (_e23 - point_8);
    const float2 _e25 = figma_cubic_point(axis_3, c_3, d_3, chord_seed);
    float2 chord_delta = (_e25 - point_8);
    t_3 = ((dot(chord_delta, chord_delta) < dot(y_delta, y_delta)) ? chord_seed : y_seed);
    while(true) {
        uint _e34 = iteration;
        if ((_e34 < 4u)) {
        } else {
            break;
        }
        {
            float _e37 = t_3;
            const float2 _e38 = figma_cubic_point(axis_3, c_3, d_3, _e37);
            float _e39 = t_3;
            const float2 _e40 = figma_cubic_derivative(axis_3, c_3, d_3, _e39);
            float _e41 = t_3;
            const float2 _e42 = figma_cubic_second_derivative(axis_3, c_3, d_3, _e41);
            float2 delta_1 = (_e38 - point_8);
            float denominator = (dot(_e40, _e40) + dot(delta_1, _e42));
            if ((abs(denominator) > FIGMA_EPSILON)) {
                float _e50 = t_3;
                t_3 = clamp((_e50 - (dot(delta_1, _e40) / denominator)), 0.0, 1.0);
            }
            uint _e58 = iteration;
            iteration = (_e58 + 1u);
        }
    }
    float _e60 = t_3;
    closest_t = _e60;
    float _e62 = t_3;
    const float2 _e63 = figma_cubic_point(axis_3, c_3, d_3, _e62);
    closest_point = _e63;
    float2 _e65 = closest_point;
    closest_distance = length((point_8 - _e65));
    float2 start = float2(0.0, 0.0);
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
    float2 _e82 = closest_point;
    float _e83 = closest_t;
    const float2 _e84 = figma_cubic_derivative(axis_3, c_3, d_3, _e83);
    float _e85 = closest_distance;
    float _e86 = closest_t;
    const CubicClosestPoint cubicclosestpoint = ConstructCubicClosestPoint(_e82, _e84, _e85, _e86);
    return cubicclosestpoint;
}

float figma_cross_2d(float2 a, float2 b)
{
    return ((a.x * b.y) - (a.y * b.x));
}

float2 figma_unfold_normal(float2 normal_1, bool mirrored)
{
    return (mirrored ? float2(-(normal_1.y), normal_1.x) : float2(normal_1.x, -(normal_1.y)));
}

SignedDistanceSample figma_corner_signed_distance(float2 corner_to_point_1, FigmaCornerParams params)
{
    SignedDistanceSample sample_2 = (SignedDistanceSample)0;

    float2 z = (corner_to_point_1 + float2(params.horizontal.p, params.vertical.p));
    if ((((params.radius <= FIGMA_EPSILON) || (z.x <= 0.0)) || (z.y <= 0.0))) {
        const SignedDistanceSample signeddistancesample_1 = ConstructSignedDistanceSample(max(corner_to_point_1.x, corner_to_point_1.y), ((corner_to_point_1.x > corner_to_point_1.y) ? float2(1.0, 0.0) : float2(0.0, 1.0)), 0.0, FIGMA_SEGMENT_STRAIGHT);
        return signeddistancesample_1;
    }
    float2 horizontal_point = float2(z.x, (params.vertical.p - z.y));
    float2 vertical_point = float2(z.y, (params.horizontal.p - z.x));
    float2 circle_center = float2((params.horizontal.p - params.radius), params.radius);
    float2 join = float2(((params.horizontal.a + params.horizontal.b) + params.c), params.d);
    float2 start_direction = ((join - circle_center) / (params.radius).xx);
    float2 to_point = (horizontal_point - circle_center);
    float to_point_length = length(to_point);
    float2 point_direction = ((to_point_length > FIGMA_EPSILON) ? (to_point / (max(to_point_length, FIGMA_EPSILON)).xx) : start_direction);
    const float _e75 = figma_cross_2d(start_direction, point_direction);
    float arc_angle = clamp(atan2(_e75, dot(start_direction, point_direction)), 0.0, params.arc_sweep);
    float arc_sine = sin(arc_angle);
    float arc_cosine = cos(arc_angle);
    float2 arc_normal = float2(((arc_cosine * start_direction.x) - (arc_sine * start_direction.y)), ((arc_sine * start_direction.x) + (arc_cosine * start_direction.y)));
    float2 arc_point = (circle_center + (params.radius * arc_normal));
    float2 arc_delta_1 = (horizontal_point - arc_point);
    float arc_t = ((params.arc_sweep > FIGMA_EPSILON) ? (arc_angle / max(params.arc_sweep, FIGMA_EPSILON)) : 0.0);
    const float _e108 = figma_signed_distance(arc_delta_1, arc_normal, length(arc_delta_1));
    const float2 _e110 = figma_unfold_normal(arc_normal, false);
    sample_2 = ConstructSignedDistanceSample(_e108, _e110, arc_t, FIGMA_SEGMENT_ARC);
    if ((params.smoothing > FIGMA_EPSILON)) {
        float2 horizontal_bounds_delta = max(float2(0.0, 0.0), max(-(horizontal_point), (horizontal_point - join)));
        float _e126 = sample_2.distance_;
        float _e128 = sample_2.distance_;
        if ((dot(horizontal_bounds_delta, horizontal_bounds_delta) <= (((_e126 * _e128) * 1.000001) + FIGMA_EPSILON))) {
            const CubicClosestPoint _e138 = closest_figma_cubic(horizontal_point, params.horizontal, params.c, params.d);
            float2 normal_3 = normalize(float2(_e138.tangent.y, -(_e138.tangent.x)));
            const float _e149 = figma_signed_distance((horizontal_point - _e138.point_), normal_3, _e138.distance_);
            float _e152 = sample_2.distance_;
            if ((abs(_e149) <= abs(_e152))) {
                sample_2.distance_ = _e149;
                const float2 _e158 = figma_unfold_normal(normal_3, false);
                sample_2.normal = _e158;
                sample_2.path_t = _e138.path_t;
                sample_2.segment = FIGMA_SEGMENT_FIRST_CUBIC;
            }
        }
        float2 vertical_join = float2(((params.vertical.a + params.vertical.b) + params.c), params.d);
        float2 vertical_bounds_delta = max(float2(0.0, 0.0), max(-(vertical_point), (vertical_point - vertical_join)));
        float _e181 = sample_2.distance_;
        float _e183 = sample_2.distance_;
        if ((dot(vertical_bounds_delta, vertical_bounds_delta) <= (((_e181 * _e183) * 1.000001) + FIGMA_EPSILON))) {
            const CubicClosestPoint _e193 = closest_figma_cubic(vertical_point, params.vertical, params.c, params.d);
            float2 normal_4 = normalize(float2(_e193.tangent.y, -(_e193.tangent.x)));
            const float _e204 = figma_signed_distance((vertical_point - _e193.point_), normal_4, _e193.distance_);
            float _e207 = sample_2.distance_;
            if ((abs(_e204) <= abs(_e207))) {
                sample_2.distance_ = _e204;
                const float2 _e213 = figma_unfold_normal(normal_4, true);
                sample_2.normal = _e213;
                sample_2.path_t = _e193.path_t;
                sample_2.segment = FIGMA_SEGMENT_SECOND_CUBIC;
            }
        }
    }
    SignedDistanceSample _e218 = sample_2;
    const SignedDistanceSample signeddistancesample_2 = _e218;
    return signeddistancesample_2;
}

float figma_cubic_length(FigmaCornerParams params_1, FigmaAxisParams axis_4, float end_t)
{
    float center_ = (clamp(end_t, 0.0, 1.0) / 2.0);
    if (((center_ <= 0.0) || (params_1.smoothing <= 0.0))) {
        return 0.0;
    }
    float offset1_ = (center_ * 0.5384693);
    float offset2_ = (center_ * 0.90617985);
    const float2 _e21 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, center_);
    float speed0_ = length(_e21);
    const float2 _e26 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center_ - offset1_));
    const float2 _e31 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center_ + offset1_));
    float speed1_ = (length(_e26) + length(_e31));
    const float2 _e37 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center_ - offset2_));
    const float2 _e42 = figma_cubic_derivative(axis_4, params_1.c, params_1.d, (center_ + offset2_));
    float speed2_ = (length(_e37) + length(_e42));
    return (center_ * (((0.5688889 * speed0_) + (0.47862867 * speed1_)) + (0.23692688 * speed2_)));
}

float figma_corner_length(FigmaCornerParams params_2)
{
    const float _e3 = figma_cubic_length(params_2, params_2.horizontal, 1.0);
    const float _e10 = figma_cubic_length(params_2, params_2.vertical, 1.0);
    return ((_e3 + (params_2.radius * params_2.arc_sweep)) + _e10);
}

float figma_corner_progress(FigmaCornerParams params_3, SignedDistanceSample sample_3, float total_length)
{
    if ((sample_3.segment == FIGMA_SEGMENT_FIRST_CUBIC)) {
        const float _e8 = figma_cubic_length(params_3, params_3.horizontal, sample_3.path_t);
        return _e8;
    }
    if ((sample_3.segment == FIGMA_SEGMENT_ARC)) {
        const float _e14 = figma_cubic_length(params_3, params_3.horizontal, 1.0);
        return (_e14 + ((params_3.radius * params_3.arc_sweep) * sample_3.path_t));
    }
    if ((sample_3.segment == FIGMA_SEGMENT_SECOND_CUBIC)) {
        const float _e26 = figma_cubic_length(params_3, params_3.vertical, sample_3.path_t);
        return (total_length - _e26);
    }
    return 0.0;
}

bool figma_has_corner_candidate(float2 point_9, float2 size_7, float4 horizontal_extents_1, float4 vertical_extents_1)
{
    const bool _e7 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.x, vertical_extents_1.x, 0u);
    const bool _e11 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.y, vertical_extents_1.y, 1u);
    const bool _e16 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.z, vertical_extents_1.z, 2u);
    const bool _e21 = figma_is_corner_candidate(point_9, size_7, horizontal_extents_1.w, vertical_extents_1.w, 3u);
    return (((_e7 || _e11) || _e16) || _e21);
}

float2 figma_orient_corner_normal(float2 normal_2, uint corner_3)
{
    switch(corner_3) {
        case 0u: {
            return -(normal_2);
        }
        case 1u: {
            return float2(normal_2.x, -(normal_2.y));
        }
        case 2u: {
            return normal_2;
        }
        default: {
            return float2(-(normal_2.x), normal_2.y);
        }
    }
}

FigmaRectangleSample ConstructFigmaRectangleSample(SignedDistanceSample arg0, uint arg1) {
    FigmaRectangleSample ret = (FigmaRectangleSample)0;
    ret.signed_distance = arg0;
    ret.corner = arg1;
    return ret;
}

FigmaRectangleSample figma_smooth_rectangle_sample(float2 point_10, Bounds bounds_10, Corners corner_radii_9, float4 horizontal_reaches, float4 vertical_reaches, float4 smoothing_factors_1)
{
    FigmaRectangleSample result = (FigmaRectangleSample)0;
    uint corner_4 = 0u;
    SignedDistanceSample candidate = (SignedDistanceSample)0;

    float2 local_point_1 = (point_10 - bounds_10.origin);
    const float4 _e8 = corner_values(corner_radii_9);
    const SignedDistanceSample _e10 = figma_nearest_straight_sample(local_point_1, bounds_10.size, horizontal_reaches, vertical_reaches);
    result = ConstructFigmaRectangleSample(_e10, FIGMA_NO_CORNER);
    const bool _e15 = figma_has_corner_candidate(local_point_1, bounds_10.size, horizontal_reaches, vertical_reaches);
    if (!(_e15)) {
        FigmaRectangleSample _e17 = result;
        const FigmaRectangleSample figmarectanglesample = _e17;
        return figmarectanglesample;
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
            const bool _e29 = figma_is_corner_candidate(local_point_1, bounds_10.size, horizontal_reaches[min(uint(_e24), 3u)], vertical_reaches[min(uint(_e26), 3u)], _e28);
            if (_e29) {
                uint _e30 = corner_4;
                uint _e32 = corner_4;
                uint _e34 = corner_4;
                const FigmaCornerParams _e36 = figma_corner_params(_e8[min(uint(_e30), 3u)], horizontal_reaches[min(uint(_e32), 3u)], vertical_reaches[min(uint(_e34), 3u)], smoothing_factors_1);
                if ((_e36.radius > FIGMA_EPSILON)) {
                    uint _e41 = corner_4;
                    const float2 _e42 = figma_corner_to_point(local_point_1, bounds_10.size, _e41);
                    const SignedDistanceSample _e43 = figma_corner_signed_distance(_e42, _e36);
                    candidate = _e43;
                    float2 _e47 = candidate.normal;
                    uint _e48 = corner_4;
                    const float2 _e49 = figma_orient_corner_normal(_e47, _e48);
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
            uint _e63 = corner_4;
            corner_4 = (_e63 + 1u);
        }
    }
    FigmaRectangleSample _e65 = result;
    const FigmaRectangleSample figmarectanglesample_1 = _e65;
    return figmarectanglesample_1;
}

float prepared_corner_signed_distance(float2 point_11, Bounds bounds_11, Corners corner_radii_10, float corner_smoothing_9, PreparedCorners prepared_4)
{
    if ((prepared_4.superellipse_power > 0.0)) {
        const float _e10 = normalized_superellipse_signed_distance(point_11, bounds_11, corner_radii_10, corner_smoothing_9, prepared_4.horizontal_reaches, prepared_4.superellipse_power);
        return _e10;
    }
    if ((prepared_4.smoothing_factors.x <= 0.0)) {
        const float _e15 = rounded_rectangle_signed_distance(point_11, bounds_11, corner_radii_10);
        return _e15;
    }
    const FigmaRectangleSample _e19 = figma_smooth_rectangle_sample(point_11, bounds_11, corner_radii_10, prepared_4.horizontal_reaches, prepared_4.vertical_reaches, prepared_4.smoothing_factors);
    return _e19.signed_distance.distance_;
}

QuadGeometry ConstructQuadGeometry(float2 arg0, float2 arg1, float arg2, float2 arg3, float2 arg4, float2 arg5, bool arg6, bool arg7) {
    QuadGeometry ret = (QuadGeometry)0;
    ret.point_ = arg0;
    ret.center_to_point = arg1;
    ret.corner_radius = arg2;
    ret.corner_center_to_point = arg3;
    ret.reduced_border = arg4;
    ret.straight_border_inner_corner_to_point = arg5;
    ret.near_rounded_corner = arg6;
    ret.unrounded = arg7;
    return ret;
}

QuadGeometry quad_geometry(Quad quad, float2 position_12)
{
    const float2 _e3 = Bounds_1half_size(quad.bounds);
    float2 point_12 = (position_12 - quad.bounds.origin);
    float2 center_to_point_3 = (point_12 - _e3);
    const float _e9 = pick_corner_radius(center_to_point_3, quad.corner_radii);
    float2 corner_to_point_3 = (abs(center_to_point_3) - _e3);
    float2 corner_center_to_point_2 = (corner_to_point_3 + (_e9).xx);
    float2 border_2 = float2(((center_to_point_3.x < 0.0) ? quad.border_widths.left : quad.border_widths.right), ((center_to_point_3.y < 0.0) ? quad.border_widths.top : quad.border_widths.bottom));
    float2 reduced_border = float2(((border_2.x == 0.0) ? -0.5 : border_2.x), ((border_2.y == 0.0) ? -0.5 : border_2.y));
    const bool _e53 = Corners_1is_zero(quad.corner_radii);
    const QuadGeometry quadgeometry = ConstructQuadGeometry(point_12, center_to_point_3, _e9, corner_center_to_point_2, reduced_border, (corner_to_point_3 + reduced_border), ((corner_center_to_point_2.x >= 0.0) && (corner_center_to_point_2.y >= 0.0)), _e53);
    return quadgeometry;
}

bool is_unaffected_background(QuadGeometry geometry)
{
    return (((geometry.straight_border_inner_corner_to_point.x < -0.5) && (geometry.straight_border_inner_corner_to_point.y < -0.5)) && !(geometry.near_rounded_corner));
}

float inner_border_signed_distance(QuadGeometry geometry_1, float outer_signed_distance)
{
    if (((geometry_1.corner_center_to_point.x <= 0.0) || (geometry_1.corner_center_to_point.y <= 0.0))) {
        return -(max(geometry_1.straight_border_inner_corner_to_point.x, geometry_1.straight_border_inner_corner_to_point.y));
    }
    if (((geometry_1.straight_border_inner_corner_to_point.x > 0.0) || (geometry_1.straight_border_inner_corner_to_point.y > 0.0))) {
        return DEFINITELY_OUTSIDE_INNER_BORDER;
    }
    if ((geometry_1.reduced_border.x == geometry_1.reduced_border.y)) {
        return -((outer_signed_distance + geometry_1.reduced_border.x));
    }
    float2 ellipse_radii = max(float2(0.0, 0.0), ((geometry_1.corner_radius).xx - geometry_1.reduced_border));
    const float _e45 = quarter_ellipse_signed_distance(geometry_1.corner_center_to_point, ellipse_radii);
    return _e45;
}

BorderDistances ConstructBorderDistances(float arg0, float arg1) {
    BorderDistances ret = (BorderDistances)0;
    ret.outer = arg0;
    ret.inner = arg1;
    return ret;
}

BorderDistances border_distances(QuadGeometry geometry_2)
{
    const float _e3 = rounded_rectangle_signed_distance_from_corner(geometry_2.corner_center_to_point, geometry_2.corner_radius);
    const float _e4 = inner_border_signed_distance(geometry_2, _e3);
    const BorderDistances borderdistances = ConstructBorderDistances(_e3, _e4);
    return borderdistances;
}

float dash_period_per_border_width(Quad quad_1)
{
    return (max(quad_1.border_dashed_length, 0.0) + max(quad_1.border_dashed_gap, 0.0));
}

float dash_velocity_1(float border_width, float period_per_border_width)
{
    if (((border_width <= 0.0) || (period_per_border_width <= 0.0))) {
        return 0.0;
    } else {
        return ((1.0 / period_per_border_width) / border_width);
    }
}

DashPosition ConstructDashPosition(float arg0, float arg1, float arg2) {
    DashPosition ret = (DashPosition)0;
    ret.position = arg0;
    ret.perimeter = arg1;
    ret.velocity = arg2;
    return ret;
}

DashPosition straight_dash_position(Quad quad_2, QuadGeometry geometry_3)
{
    bool horizontal_2 = (geometry_3.corner_center_to_point.x < geometry_3.corner_center_to_point.y);
    float border_width_1 = (horizontal_2 ? max(quad_2.border_widths.bottom, quad_2.border_widths.top) : max(quad_2.border_widths.right, quad_2.border_widths.left));
    const float _e18 = dash_period_per_border_width(quad_2);
    const float _e19 = dash_velocity_1(border_width_1, _e18);
    const DashPosition dashposition = ConstructDashPosition(((horizontal_2 ? geometry_3.point_.x : geometry_3.point_.y) * _e19), ((horizontal_2 ? quad_2.bounds.size.x : quad_2.bounds.size.y) * _e19), _e19);
    return dashposition;
}

Edges ConstructEdges(float arg0, float arg1, float arg2, float arg3) {
    Edges ret = (Edges)0;
    ret.top = arg0;
    ret.right = arg1;
    ret.bottom = arg2;
    ret.left = arg3;
    return ret;
}

Edges side_dash_velocities(Edges border_widths, float period_per_border_width_1)
{
    const float _e3 = dash_velocity_1(border_widths.top, period_per_border_width_1);
    const float _e5 = dash_velocity_1(border_widths.right, period_per_border_width_1);
    const float _e7 = dash_velocity_1(border_widths.bottom, period_per_border_width_1);
    const float _e9 = dash_velocity_1(border_widths.left, period_per_border_width_1);
    const Edges edges_1 = ConstructEdges(_e3, _e5, _e7, _e9);
    return edges_1;
}

Edges straight_side_dash_lengths(Bounds bounds_12, Corners radii_4, Edges velocities)
{
    const Edges edges_2 = ConstructEdges((((bounds_12.size.x - radii_4.top_left) - radii_4.top_right) * velocities.top), (((bounds_12.size.y - radii_4.top_right) - radii_4.bottom_right) * velocities.right), (((bounds_12.size.x - radii_4.bottom_right) - radii_4.bottom_left) * velocities.bottom), (((bounds_12.size.y - radii_4.bottom_left) - radii_4.top_left) * velocities.left));
    return edges_2;
}

Corners ConstructCorners(float arg0, float arg1, float arg2, float arg3) {
    Corners ret = (Corners)0;
    ret.top_left = arg0;
    ret.top_right = arg1;
    ret.bottom_right = arg2;
    ret.bottom_left = arg3;
    return ret;
}

Corners corner_dash_velocities(Edges side_velocities)
{
    const float _e3 = corner_dash_velocity(side_velocities.top, side_velocities.left);
    const float _e6 = corner_dash_velocity(side_velocities.top, side_velocities.right);
    const float _e9 = corner_dash_velocity(side_velocities.bottom, side_velocities.right);
    const float _e12 = corner_dash_velocity(side_velocities.bottom, side_velocities.left);
    const Corners corners_1 = ConstructCorners(_e3, _e6, _e9, _e12);
    return corners_1;
}

Corners corner_dash_lengths(Corners radii_5, Corners velocities_1)
{
    const Corners corners_2 = ConstructCorners(((radii_5.top_left * 1.5707964) * velocities_1.top_left), ((radii_5.top_right * 1.5707964) * velocities_1.top_right), ((radii_5.bottom_right * 1.5707964) * velocities_1.bottom_right), ((radii_5.bottom_left * 1.5707964) * velocities_1.bottom_left));
    return corners_2;
}

RoundedDashLayout ConstructRoundedDashLayout(Edges arg0, Corners arg1, float arg2, float arg3, float arg4, float arg5, float arg6, float arg7) {
    RoundedDashLayout ret = (RoundedDashLayout)0;
    ret.side_velocities = arg0;
    ret.corner_velocities = arg1;
    ret.right_start = arg2;
    ret.bottom_right_start = arg3;
    ret.bottom_left_start = arg4;
    ret.left_start = arg5;
    ret.top_left_start = arg6;
    ret.perimeter = arg7;
    return ret;
}

RoundedDashLayout perimeter_dash_layout(Edges side_velocities_1, Edges side_lengths, Corners corner_velocities, float4 corner_lengths)
{
    float right_start = (side_lengths.top + corner_lengths.y);
    float bottom_right_start = (right_start + side_lengths.right);
    float bottom_left_start = ((bottom_right_start + corner_lengths.z) + side_lengths.bottom);
    float left_start = (bottom_left_start + corner_lengths.w);
    float top_left_start = (left_start + side_lengths.left);
    const RoundedDashLayout roundeddashlayout = ConstructRoundedDashLayout(side_velocities_1, corner_velocities, right_start, bottom_right_start, bottom_left_start, left_start, top_left_start, (top_left_start + corner_lengths.x));
    return roundeddashlayout;
}

RoundedDashLayout rounded_dash_layout(Quad quad_3)
{
    const float _e2 = dash_period_per_border_width(quad_3);
    const Edges _e3 = side_dash_velocities(quad_3.border_widths, _e2);
    const Edges _e6 = straight_side_dash_lengths(quad_3.bounds, quad_3.corner_radii, _e3);
    const Corners _e7 = corner_dash_velocities(_e3);
    const Corners _e9 = corner_dash_lengths(quad_3.corner_radii, _e7);
    const float4 _e10 = corner_values(_e9);
    const RoundedDashLayout _e11 = perimeter_dash_layout(_e3, _e6, _e7, _e10);
    const RoundedDashLayout roundeddashlayout_1 = _e11;
    return roundeddashlayout_1;
}

float4 smoothed_corner_lengths(Quad quad_4, PreparedCorners prepared_5)
{
    if (((quad_4.border_style != BorderStyle_Dashed) || (quad_4.corner_smoothing <= 0.0))) {
        const float4 _e10 = corner_values(quad_4.corner_radii);
        return (_e10 * 1.5707964);
    }
    const float4 _e14 = corner_values(quad_4.corner_radii);
    const FigmaCornerParams _e21 = figma_corner_params(_e14.x, prepared_5.horizontal_reaches.x, prepared_5.vertical_reaches.x, prepared_5.smoothing_factors);
    const float _e22 = figma_corner_length(_e21);
    const FigmaCornerParams _e29 = figma_corner_params(_e14.y, prepared_5.horizontal_reaches.y, prepared_5.vertical_reaches.y, prepared_5.smoothing_factors);
    const float _e30 = figma_corner_length(_e29);
    const FigmaCornerParams _e37 = figma_corner_params(_e14.z, prepared_5.horizontal_reaches.z, prepared_5.vertical_reaches.z, prepared_5.smoothing_factors);
    const float _e38 = figma_corner_length(_e37);
    const FigmaCornerParams _e45 = figma_corner_params(_e14.w, prepared_5.horizontal_reaches.w, prepared_5.vertical_reaches.w, prepared_5.smoothing_factors);
    const float _e46 = figma_corner_length(_e45);
    return float4(_e22, _e30, _e38, _e46);
}

RoundedDashLayout smoothed_dash_layout(Quad quad_5, PreparedCorners prepared_6, float4 corner_lengths_1)
{
    const float _e4 = dash_period_per_border_width(quad_5);
    const Edges _e5 = side_dash_velocities(quad_5.border_widths, _e4);
    Edges side_lengths_1 = ConstructEdges((((quad_5.bounds.size.x - prepared_6.horizontal_reaches.x) - prepared_6.horizontal_reaches.y) * _e5.top), (((quad_5.bounds.size.y - prepared_6.vertical_reaches.y) - prepared_6.vertical_reaches.z) * _e5.right), (((quad_5.bounds.size.x - prepared_6.horizontal_reaches.z) - prepared_6.horizontal_reaches.w) * _e5.bottom), (((quad_5.bounds.size.y - prepared_6.vertical_reaches.w) - prepared_6.vertical_reaches.x) * _e5.left));
    const Corners _e51 = corner_dash_velocities(_e5);
    const float4 _e52 = corner_values(_e51);
    const RoundedDashLayout _e54 = perimeter_dash_layout(_e5, side_lengths_1, _e51, (corner_lengths_1 * _e52));
    const RoundedDashLayout roundeddashlayout_2 = _e54;
    return roundeddashlayout_2;
}

float smoothed_dashed_border_alpha(Quad quad_6, QuadGeometry geometry_4, FigmaRectangleSample rectangle_sample, PreparedCorners prepared_7, float4 corner_lengths_2, float2 border_1, float2 straight_border_inner_corner_to_point)
{
    float dash_position = 0.0;
    float dash_velocity_2 = 0.0;
    bool horizontal = (bool)0;
    float straight_width = (float)0;

    const float _e7 = dash_period_per_border_width(quad_6);
    if ((_e7 <= 0.0)) {
        return 1.0;
    }
    const RoundedDashLayout _e11 = smoothed_dash_layout(quad_6, prepared_7, corner_lengths_2);
    if (((rectangle_sample.corner != FIGMA_NO_CORNER) && (rectangle_sample.signed_distance.segment != FIGMA_SEGMENT_STRAIGHT))) {
        uint corner_idx = rectangle_sample.corner;
        const float4 _e26 = corner_values(quad_6.corner_radii);
        float corner_length = corner_lengths_2[min(uint(corner_idx), 3u)];
        const FigmaCornerParams _e34 = figma_corner_params(_e26[min(uint(corner_idx), 3u)], prepared_7.horizontal_reaches[min(uint(corner_idx), 3u)], prepared_7.vertical_reaches[min(uint(corner_idx), 3u)], prepared_7.smoothing_factors);
        const float _e36 = figma_corner_progress(_e34, rectangle_sample.signed_distance, corner_length);
        switch(corner_idx) {
            case 0u: {
                dash_velocity_2 = _e11.corner_velocities.top_left;
                float _e41 = dash_velocity_2;
                dash_position = (_e11.top_left_start + ((corner_length - _e36) * _e41));
                break;
            }
            case 1u: {
                dash_velocity_2 = _e11.corner_velocities.top_right;
                float _e47 = dash_velocity_2;
                dash_position = (_e11.right_start + (_e36 * _e47));
                break;
            }
            case 2u: {
                dash_velocity_2 = _e11.corner_velocities.bottom_right;
                float _e54 = dash_velocity_2;
                dash_position = (_e11.bottom_right_start + ((corner_length - _e36) * _e54));
                break;
            }
            default: {
                dash_velocity_2 = _e11.corner_velocities.bottom_left;
                float _e60 = dash_velocity_2;
                dash_position = (_e11.bottom_left_start + (_e36 * _e60));
                break;
            }
        }
    } else {
        horizontal = (straight_border_inner_corner_to_point.x < straight_border_inner_corner_to_point.y);
        bool _e69 = horizontal;
        straight_width = (_e69 ? border_1.y : border_1.x);
        float _e72 = straight_width;
        if ((_e72 <= 0.0)) {
            bool _e75 = horizontal;
            horizontal = !(_e75);
            bool _e79 = horizontal;
            straight_width = (_e79 ? border_1.y : border_1.x);
        }
        bool _e81 = horizontal;
        if (_e81) {
            if ((geometry_4.center_to_point.y < 0.0)) {
                dash_velocity_2 = _e11.side_velocities.top;
                float _e93 = dash_velocity_2;
                dash_position = ((geometry_4.point_.x - prepared_7.horizontal_reaches.x) * _e93);
            } else {
                dash_velocity_2 = _e11.side_velocities.bottom;
                float _e103 = dash_velocity_2;
                dash_position = (_e11.bottom_left_start - ((geometry_4.point_.x - prepared_7.horizontal_reaches.w) * _e103));
            }
        } else {
            if ((geometry_4.center_to_point.x < 0.0)) {
                dash_velocity_2 = _e11.side_velocities.left;
                float _e118 = dash_velocity_2;
                dash_position = (_e11.top_left_start - ((geometry_4.point_.y - prepared_7.vertical_reaches.x) * _e118));
            } else {
                dash_velocity_2 = _e11.side_velocities.right;
                float _e129 = dash_velocity_2;
                dash_position = (_e11.right_start + ((geometry_4.point_.y - prepared_7.vertical_reaches.y) * _e129));
            }
        }
    }
    float dash_length_1 = (max(quad_6.border_dashed_length, 0.0) / _e7);
    if ((_e11.perimeter >= 1.0)) {
        float period_2 = (_e11.perimeter / floor(_e11.perimeter));
        float _e143 = dash_position;
        float _e144 = dash_velocity_2;
        const float _e145 = dash_coverage(_e143, period_2, dash_length_1, _e144);
        return _e145;
    } else {
        return 1.0;
    }
}

DashPosition rounded_dash_position(Quad quad_7, QuadGeometry geometry_5)
{
    Corners radii_8 = quad_7.corner_radii;
    const RoundedDashLayout _e3 = rounded_dash_layout(quad_7);
    bool horizontal_3 = (geometry_5.corner_center_to_point.x < geometry_5.corner_center_to_point.y);
    bool on_right = (geometry_5.center_to_point.x >= 0.0);
    bool on_bottom = (geometry_5.center_to_point.y >= 0.0);
    float top_position = ((geometry_5.point_.x - radii_8.top_left) * _e3.side_velocities.top);
    float right_position = (_e3.right_start + ((geometry_5.point_.y - radii_8.top_right) * _e3.side_velocities.right));
    float bottom_position = (_e3.bottom_left_start - ((geometry_5.point_.x - radii_8.bottom_left) * _e3.side_velocities.bottom));
    float left_position = (_e3.top_left_start - ((geometry_5.point_.y - radii_8.top_left) * _e3.side_velocities.left));
    float horizontal_position_1 = (on_bottom ? bottom_position : top_position);
    float vertical_position_1 = (on_right ? right_position : left_position);
    float side_position = (horizontal_3 ? horizontal_position_1 : vertical_position_1);
    float horizontal_velocity = (on_bottom ? _e3.side_velocities.bottom : _e3.side_velocities.top);
    float vertical_velocity = (on_right ? _e3.side_velocities.right : _e3.side_velocities.left);
    float side_velocity = (horizontal_3 ? horizontal_velocity : vertical_velocity);
    if (geometry_5.near_rounded_corner) {
        float corner_position = (atan2(geometry_5.corner_center_to_point.y, geometry_5.corner_center_to_point.x) * geometry_5.corner_radius);
        float top_right_position = (_e3.right_start - (corner_position * _e3.corner_velocities.top_right));
        float bottom_right_position = (_e3.bottom_right_start + (corner_position * _e3.corner_velocities.bottom_right));
        float bottom_left_position = (_e3.left_start - (corner_position * _e3.corner_velocities.bottom_left));
        float top_left_position = (_e3.top_left_start + (corner_position * _e3.corner_velocities.top_left));
        float right_position_1 = (on_bottom ? bottom_right_position : top_right_position);
        float left_position_1 = (on_bottom ? bottom_left_position : top_left_position);
        float position_17 = (on_right ? right_position_1 : left_position_1);
        float right_velocity = (on_bottom ? _e3.corner_velocities.bottom_right : _e3.corner_velocities.top_right);
        float left_velocity = (on_bottom ? _e3.corner_velocities.bottom_left : _e3.corner_velocities.top_left);
        const DashPosition dashposition_1 = ConstructDashPosition(position_17, _e3.perimeter, (on_right ? right_velocity : left_velocity));
        return dashposition_1;
    }
    const DashPosition dashposition_2 = ConstructDashPosition(side_position, _e3.perimeter, side_velocity);
    return dashposition_2;
}

float dashed_border_alpha(Quad quad_8, QuadGeometry geometry_6)
{
    DashPosition dash = ConstructDashPosition(0.0, 0.0, 0.0);

    const float _e2 = dash_period_per_border_width(quad_8);
    if ((_e2 <= 0.0)) {
        return 1.0;
    }
    if (geometry_6.unrounded) {
        const DashPosition _e12 = straight_dash_position(quad_8, geometry_6);
        dash = _e12;
    } else {
        const DashPosition _e13 = rounded_dash_position(quad_8, geometry_6);
        dash = _e13;
    }
    float dash_length_2 = (max(quad_8.border_dashed_length, 0.0) / _e2);
    float _e19 = dash.perimeter;
    float perimeter = (_e19 - (geometry_6.unrounded ? dash_length_2 : 0.0));
    if ((perimeter >= 1.0)) {
        float period_3 = (perimeter / floor(perimeter));
        float _e29 = dash.position;
        float _e31 = dash.velocity;
        const float _e32 = dash_coverage(_e29, period_3, dash_length_2, _e31);
        return _e32;
    } else {
        if ((geometry_6.unrounded && (perimeter > dash_length_2))) {
            float _e37 = dash.position;
            float _e39 = dash.velocity;
            const float _e40 = dash_coverage(_e37, perimeter, dash_length_2, _e39);
            return _e40;
        } else {
            return 1.0;
        }
    }
}

QuadVertexData ConstructQuadVertexData(float4 arg0, PreparedPaint arg1, uint arg2, float4 arg3, PreparedPaint arg4) {
    QuadVertexData ret = (QuadVertexData)0;
    ret.position = arg0;
    ret.border = arg1;
    ret.quad_id = arg2;
    ret.clip_distances = arg3;
    ret.fill = arg4;
    return ret;
}

QuadVertexData prepare_quad_vertex(uint vertex_id_6, uint instance_id_2, Quad quad_9)
{
    const RectangleVertex _e4 = rectangle_vertex(vertex_id_6, quad_9.bounds);
    const Paint _e8 = Paint_new(quad_9.border_color, quad_9.bounds);
    const PreparedPaint _e9 = prepare_paint(_e8);
    const float4 _e13 = clip_distances(_e4.viewport_position, quad_9.content_mask.bounds);
    const Paint _e16 = Paint_new(quad_9.background, quad_9.bounds);
    const PreparedPaint _e17 = prepare_paint(_e16);
    const QuadVertexData quadvertexdata = ConstructQuadVertexData(_e4.clip_position, _e9, instance_id_2, _e13, _e17);
    return quadvertexdata;
}

QuadVarying ConstructQuadVarying(float4 arg0, float4 arg1, uint arg2, float4 arg3, float4 arg4, float4 arg5, float4 arg6, float4 arg7, float4 arg8) {
    QuadVarying ret = (QuadVarying)0;
    ret.position = arg0;
    ret.border_solid = arg1;
    ret.quad_id = arg2;
    ret.clip_distances = arg3;
    ret.fill_solid = arg4;
    ret.fill_color0_ = arg5;
    ret.fill_color1_ = arg6;
    ret.border_color0_ = arg7;
    ret.border_color1_ = arg8;
    return ret;
}

Bounds ConstructBounds(float2 arg0, float2 arg1) {
    Bounds ret = (Bounds)0;
    ret.origin = arg0;
    ret.size = arg1;
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

LinearColorStop ConstructLinearColorStop(Hsla arg0, float arg1) {
    LinearColorStop ret = (LinearColorStop)0;
    ret.color = arg0;
    ret.percentage = arg1;
    return ret;
}

typedef LinearColorStop ret_Constructarray2_LinearColorStop_[2];
ret_Constructarray2_LinearColorStop_ Constructarray2_LinearColorStop_(LinearColorStop arg0, LinearColorStop arg1) {
    LinearColorStop ret[2] = { arg0, arg1 };
    return ret;
}

Background ConstructBackground(uint arg0, uint arg1, Hsla arg2, float arg3, LinearColorStop arg4[2], uint arg5) {
    Background ret = (Background)0;
    ret.tag = arg0;
    ret.color_space = arg1;
    ret.solid = arg2;
    ret.gradient_angle_or_pattern_height = arg3;
    ret.colors = arg4;
    ret.padding = arg5;
    return ret;
}

Quad ConstructQuad(uint arg0, uint arg1, float arg2, float arg3, Bounds arg4, ContentMask arg5, Background arg6, Background arg7, Corners arg8, Edges arg9, float arg10, uint arg11) {
    Quad ret = (Quad)0;
    ret.order = arg0;
    ret.border_style = arg1;
    ret.border_dashed_length = arg2;
    ret.border_dashed_gap = arg3;
    ret.bounds = arg4;
    ret.content_mask = arg5;
    ret.background = arg6;
    ret.border_color = arg7;
    ret.corner_radii = arg8;
    ret.border_widths = arg9;
    ret.corner_smoothing = arg10;
    ret.padding = arg11;
    return ret;
}

VertexOutput_vertex_quad vertex_quad(uint vertex_id : SV_VertexID, uint instance_id : SV_InstanceID)
{
    Quad quad_10 = ConstructQuad(asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+0)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+12)), ConstructBounds(asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id))*248+16+0)), asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id))*248+16+8))), ConstructContentMask(ConstructBounds(asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id))*248+32+0+0)), asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id))*248+32+0+8))), ConstructEdges(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+32+16+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+32+16+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+32+16+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+32+16+12)))), ConstructBackground(asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+0)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+4)), ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+8+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+8+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+8+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+8+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+0+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+0+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+0+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+0+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+20+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+20+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+20+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+20+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+28+20+16)))), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+64+68))), ConstructBackground(asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+0)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+4)), ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+8+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+8+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+8+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+8+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+0+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+0+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+0+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+0+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+20+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+20+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+20+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+20+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+28+20+16)))), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+136+68))), ConstructCorners(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+208+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+208+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+208+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+208+12))), ConstructEdges(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+224+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+224+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+224+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+224+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+240)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id))*248+244)));
    const QuadVertexData _e6 = prepare_quad_vertex((_NagaConstants.first_vertex + vertex_id), (_NagaConstants.first_instance + instance_id), quad_10);
    const QuadVarying quadvarying = ConstructQuadVarying(_e6.position, _e6.border.solid, _e6.quad_id, _e6.clip_distances, _e6.fill.solid, _e6.fill.color0_, _e6.fill.color1_, _e6.border.color0_, _e6.border.color1_);
    const VertexOutput_vertex_quad quadvarying_1 = { quadvarying.border_solid, quadvarying.quad_id, quadvarying.clip_distances, quadvarying.fill_solid, quadvarying.fill_color0_, quadvarying.fill_color1_, quadvarying.border_color0_, quadvarying.border_color1_, quadvarying.position };
    return quadvarying_1;
}

float4 fragment_quad(FragmentInput_fragment_quad fragmentinput_fragment_quad) : SV_Target0
{
    QuadVarying input = { fragmentinput_fragment_quad.position_14, fragmentinput_fragment_quad.border_solid_1, fragmentinput_fragment_quad.quad_id_1, fragmentinput_fragment_quad.clip_distances_2, fragmentinput_fragment_quad.fill_solid_1, fragmentinput_fragment_quad.fill_color0_1, fragmentinput_fragment_quad.fill_color1_1, fragmentinput_fragment_quad.border_color0_1, fragmentinput_fragment_quad.border_color1_1 };
    float4 color = (float4)0;
    float4 border_color = (float4)0;

    const bool _e2 = is_clipped(input.clip_distances);
    if (_e2) {
        const float4 _e3 = transparent();
        return _e3;
    }
    Quad quad_11 = ConstructQuad(asuint(QUADS.Load(uint(input.quad_id)*248+0)), asuint(QUADS.Load(uint(input.quad_id)*248+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+12)), ConstructBounds(asfloat(QUADS.Load2(uint(input.quad_id)*248+16+0)), asfloat(QUADS.Load2(uint(input.quad_id)*248+16+8))), ConstructContentMask(ConstructBounds(asfloat(QUADS.Load2(uint(input.quad_id)*248+32+0+0)), asfloat(QUADS.Load2(uint(input.quad_id)*248+32+0+8))), ConstructEdges(asfloat(QUADS.Load(uint(input.quad_id)*248+32+16+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+32+16+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+32+16+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+32+16+12)))), ConstructBackground(asuint(QUADS.Load(uint(input.quad_id)*248+64+0)), asuint(QUADS.Load(uint(input.quad_id)*248+64+4)), ConstructHsla(asfloat(QUADS.Load(uint(input.quad_id)*248+64+8+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+8+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+8+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+8+12))), asfloat(QUADS.Load(uint(input.quad_id)*248+64+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+0+0+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+0+0+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+0+0+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+0+0+12))), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+20+0+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+20+0+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+20+0+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+20+0+12))), asfloat(QUADS.Load(uint(input.quad_id)*248+64+28+20+16)))), asuint(QUADS.Load(uint(input.quad_id)*248+64+68))), ConstructBackground(asuint(QUADS.Load(uint(input.quad_id)*248+136+0)), asuint(QUADS.Load(uint(input.quad_id)*248+136+4)), ConstructHsla(asfloat(QUADS.Load(uint(input.quad_id)*248+136+8+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+8+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+8+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+8+12))), asfloat(QUADS.Load(uint(input.quad_id)*248+136+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+0+0+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+0+0+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+0+0+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+0+0+12))), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+20+0+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+20+0+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+20+0+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+20+0+12))), asfloat(QUADS.Load(uint(input.quad_id)*248+136+28+20+16)))), asuint(QUADS.Load(uint(input.quad_id)*248+136+68))), ConstructCorners(asfloat(QUADS.Load(uint(input.quad_id)*248+208+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+208+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+208+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+208+12))), ConstructEdges(asfloat(QUADS.Load(uint(input.quad_id)*248+224+0)), asfloat(QUADS.Load(uint(input.quad_id)*248+224+4)), asfloat(QUADS.Load(uint(input.quad_id)*248+224+8)), asfloat(QUADS.Load(uint(input.quad_id)*248+224+12))), asfloat(QUADS.Load(uint(input.quad_id)*248+240)), asuint(QUADS.Load(uint(input.quad_id)*248+244)));
    const float _e12 = ContentMask_alpha(quad_11.content_mask, input.position.xy);
    const Paint _e15 = Paint_new(quad_11.background, quad_11.bounds);
    const PreparedPaint _e21 = PreparedPaint_new(input.fill_solid, input.fill_color0_, input.fill_color1_);
    const float4 _e22 = paint_color(_e15, input.position.xy, _e21);
    const bool _e24 = Edges_1is_zero(quad_11.border_widths);
    const bool _e26 = Corners_1is_zero(quad_11.corner_radii);
    if ((_e24 && _e26)) {
        const float4 _e28 = blend_color(_e22, _e12);
        return _e28;
    }
    const QuadGeometry _e31 = quad_geometry(quad_11, input.position.xy);
    const bool _e32 = is_unaffected_background(_e31);
    if (_e32) {
        const float4 _e33 = blend_color(_e22, _e12);
        return _e33;
    }
    const BorderDistances _e34 = border_distances(_e31);
    color = _e22;
    if ((max(_e34.inner, _e34.outer) < PIXEL_ANTIALIAS_RADIUS)) {
        const Paint _e43 = Paint_new(quad_11.border_color, quad_11.bounds);
        const PreparedPaint _e49 = PreparedPaint_new(input.border_solid, input.border_color0_, input.border_color1_);
        const float4 _e50 = paint_color(_e43, input.position.xy, _e49);
        border_color = _e50;
        if ((quad_11.border_style == BorderStyle_Dashed)) {
            const float _e56 = dashed_border_alpha(quad_11, _e31);
            float _e57 = border_color.w;
            border_color.w = (_e57 * _e56);
        }
        float4 _e59 = border_color;
        const float4 _e60 = over(_e22, _e59);
        const float _e62 = antialiased_coverage(_e34.inner);
        color = lerp(_e22, _e60, float4(_e62, _e62, _e62, _e62));
    }
    float4 _e65 = color;
    const float _e67 = antialiased_coverage(_e34.outer);
    const float4 _e69 = blend_color(_e65, (_e67 * _e12));
    return _e69;
}

SmoothedQuadVarying ConstructSmoothedQuadVarying(float4 arg0, float4 arg1, uint arg2, float4 arg3, float4 arg4, float4 arg5, float4 arg6, float4 arg7, float4 arg8, float4 arg9, float4 arg10, float arg11, float4 arg12, float4 arg13) {
    SmoothedQuadVarying ret = (SmoothedQuadVarying)0;
    ret.position = arg0;
    ret.border_solid = arg1;
    ret.quad_id = arg2;
    ret.clip_distances = arg3;
    ret.fill_solid = arg4;
    ret.fill_color0_ = arg5;
    ret.fill_color1_ = arg6;
    ret.horizontal_corner_reaches = arg7;
    ret.vertical_corner_reaches = arg8;
    ret.corner_lengths = arg9;
    ret.smoothing_factors = arg10;
    ret.superellipse_power = arg11;
    ret.border_color0_ = arg12;
    ret.border_color1_ = arg13;
    return ret;
}

VertexOutput_vertex_smoothed_quad vertex_smoothed_quad(uint vertex_id_1 : SV_VertexID, uint instance_id_1 : SV_InstanceID)
{
    Quad quad_12 = ConstructQuad(asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+0)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+12)), ConstructBounds(asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id_1))*248+16+0)), asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id_1))*248+16+8))), ConstructContentMask(ConstructBounds(asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id_1))*248+32+0+0)), asfloat(QUADS.Load2(uint((_NagaConstants.first_instance + instance_id_1))*248+32+0+8))), ConstructEdges(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+32+16+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+32+16+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+32+16+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+32+16+12)))), ConstructBackground(asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+0)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+4)), ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+8+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+8+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+8+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+8+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+0+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+0+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+0+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+0+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+20+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+20+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+20+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+20+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+28+20+16)))), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+64+68))), ConstructBackground(asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+0)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+4)), ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+8+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+8+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+8+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+8+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+0+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+0+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+0+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+0+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+20+0+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+20+0+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+20+0+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+20+0+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+28+20+16)))), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+136+68))), ConstructCorners(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+208+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+208+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+208+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+208+12))), ConstructEdges(asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+224+0)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+224+4)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+224+8)), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+224+12))), asfloat(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+240)), asuint(QUADS.Load(uint((_NagaConstants.first_instance + instance_id_1))*248+244)));
    const QuadVertexData _e6 = prepare_quad_vertex((_NagaConstants.first_vertex + vertex_id_1), (_NagaConstants.first_instance + instance_id_1), quad_12);
    const bool _e15 = Edges_1is_zero(quad_12.border_widths);
    const PreparedCorners _e17 = prepare_corners(quad_12.bounds.size, quad_12.corner_radii, quad_12.corner_smoothing, ((quad_12.border_style == BorderStyle_Solid) && _e15));
    const float4 _e31 = smoothed_corner_lengths(quad_12, _e17);
    const SmoothedQuadVarying smoothedquadvarying = ConstructSmoothedQuadVarying(_e6.position, _e6.border.solid, _e6.quad_id, _e6.clip_distances, _e6.fill.solid, _e6.fill.color0_, _e6.fill.color1_, _e17.horizontal_reaches, _e17.vertical_reaches, _e31, _e17.smoothing_factors, _e17.superellipse_power, _e6.border.color0_, _e6.border.color1_);
    const VertexOutput_vertex_smoothed_quad smoothedquadvarying_1 = { smoothedquadvarying.border_solid, smoothedquadvarying.quad_id, smoothedquadvarying.clip_distances, smoothedquadvarying.fill_solid, smoothedquadvarying.fill_color0_, smoothedquadvarying.fill_color1_, smoothedquadvarying.horizontal_corner_reaches, smoothedquadvarying.vertical_corner_reaches, smoothedquadvarying.corner_lengths, smoothedquadvarying.smoothing_factors, smoothedquadvarying.superellipse_power, smoothedquadvarying.border_color0_, smoothedquadvarying.border_color1_, smoothedquadvarying.position };
    return smoothedquadvarying_1;
}

float4 fragment_smoothed_quad(FragmentInput_fragment_smoothed_quad fragmentinput_fragment_smoothed_quad) : SV_Target0
{
    SmoothedQuadVarying input_1 = { fragmentinput_fragment_smoothed_quad.position_16, fragmentinput_fragment_smoothed_quad.border_solid_3, fragmentinput_fragment_smoothed_quad.quad_id_3, fragmentinput_fragment_smoothed_quad.clip_distances_4, fragmentinput_fragment_smoothed_quad.fill_solid_3, fragmentinput_fragment_smoothed_quad.fill_color0_3, fragmentinput_fragment_smoothed_quad.fill_color1_3, fragmentinput_fragment_smoothed_quad.horizontal_corner_reaches_1, fragmentinput_fragment_smoothed_quad.vertical_corner_reaches_1, fragmentinput_fragment_smoothed_quad.corner_lengths_4, fragmentinput_fragment_smoothed_quad.smoothing_factors_3, fragmentinput_fragment_smoothed_quad.superellipse_power_1, fragmentinput_fragment_smoothed_quad.border_color0_3, fragmentinput_fragment_smoothed_quad.border_color1_3 };
    float2 border = (float2)0;
    float inner = 0.0;
    float4 color_1 = (float4)0;
    float4 border_color_1 = (float4)0;

    const bool _e2 = is_clipped(input_1.clip_distances);
    if (_e2) {
        const float4 _e3 = transparent();
        return _e3;
    }
    Quad quad_13 = ConstructQuad(asuint(QUADS.Load(uint(input_1.quad_id)*248+0)), asuint(QUADS.Load(uint(input_1.quad_id)*248+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+12)), ConstructBounds(asfloat(QUADS.Load2(uint(input_1.quad_id)*248+16+0)), asfloat(QUADS.Load2(uint(input_1.quad_id)*248+16+8))), ConstructContentMask(ConstructBounds(asfloat(QUADS.Load2(uint(input_1.quad_id)*248+32+0+0)), asfloat(QUADS.Load2(uint(input_1.quad_id)*248+32+0+8))), ConstructEdges(asfloat(QUADS.Load(uint(input_1.quad_id)*248+32+16+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+32+16+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+32+16+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+32+16+12)))), ConstructBackground(asuint(QUADS.Load(uint(input_1.quad_id)*248+64+0)), asuint(QUADS.Load(uint(input_1.quad_id)*248+64+4)), ConstructHsla(asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+8+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+8+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+8+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+8+12))), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+0+0+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+0+0+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+0+0+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+0+0+12))), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+20+0+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+20+0+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+20+0+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+20+0+12))), asfloat(QUADS.Load(uint(input_1.quad_id)*248+64+28+20+16)))), asuint(QUADS.Load(uint(input_1.quad_id)*248+64+68))), ConstructBackground(asuint(QUADS.Load(uint(input_1.quad_id)*248+136+0)), asuint(QUADS.Load(uint(input_1.quad_id)*248+136+4)), ConstructHsla(asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+8+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+8+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+8+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+8+12))), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+24)), Constructarray2_LinearColorStop_(ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+0+0+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+0+0+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+0+0+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+0+0+12))), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+0+16))), ConstructLinearColorStop(ConstructHsla(asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+20+0+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+20+0+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+20+0+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+20+0+12))), asfloat(QUADS.Load(uint(input_1.quad_id)*248+136+28+20+16)))), asuint(QUADS.Load(uint(input_1.quad_id)*248+136+68))), ConstructCorners(asfloat(QUADS.Load(uint(input_1.quad_id)*248+208+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+208+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+208+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+208+12))), ConstructEdges(asfloat(QUADS.Load(uint(input_1.quad_id)*248+224+0)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+224+4)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+224+8)), asfloat(QUADS.Load(uint(input_1.quad_id)*248+224+12))), asfloat(QUADS.Load(uint(input_1.quad_id)*248+240)), asuint(QUADS.Load(uint(input_1.quad_id)*248+244)));
    const float _e12 = ContentMask_alpha(quad_13.content_mask, input_1.position.xy);
    const Paint _e15 = Paint_new(quad_13.background, quad_13.bounds);
    const PreparedPaint _e21 = PreparedPaint_new(input_1.fill_solid, input_1.fill_color0_, input_1.fill_color1_);
    const float4 _e22 = paint_color(_e15, input_1.position.xy, _e21);
    PreparedCorners prepared_8 = ConstructPreparedCorners(input_1.horizontal_corner_reaches, input_1.vertical_corner_reaches, input_1.smoothing_factors, input_1.superellipse_power);
    const bool _e29 = Edges_1is_zero(quad_13.border_widths);
    if (_e29) {
        const float _e35 = prepared_corner_signed_distance(input_1.position.xy, quad_13.bounds, quad_13.corner_radii, quad_13.corner_smoothing, prepared_8);
        const float _e36 = antialiased_coverage(_e35);
        const float4 _e38 = blend_color(_e22, (_e36 * _e12));
        return _e38;
    }
    const QuadGeometry _e41 = quad_geometry(quad_13, input_1.position.xy);
    const FigmaRectangleSample _e49 = figma_smooth_rectangle_sample(input_1.position.xy, quad_13.bounds, quad_13.corner_radii, prepared_8.horizontal_reaches, prepared_8.vertical_reaches, prepared_8.smoothing_factors);
    border = float2(((_e41.center_to_point.x < 0.0) ? quad_13.border_widths.left : quad_13.border_widths.right), ((_e41.center_to_point.y < 0.0) ? quad_13.border_widths.top : quad_13.border_widths.bottom));
    switch(_e49.corner) {
        case 0u: {
            border = float2(quad_13.border_widths.left, quad_13.border_widths.top);
            break;
        }
        case 1u: {
            border = float2(quad_13.border_widths.right, quad_13.border_widths.top);
            break;
        }
        case 2u: {
            border = float2(quad_13.border_widths.right, quad_13.border_widths.bottom);
            break;
        }
        case 3u: {
            border = float2(quad_13.border_widths.left, quad_13.border_widths.bottom);
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
    float2 reduced_border_1 = float2(((_e95 == 0.0) ? -0.5 : _e92), ((_e103 == 0.0) ? -0.5 : _e100));
    const float2 _e109 = Bounds_1half_size(quad_13.bounds);
    float2 corner_to_point_4 = (abs(_e41.center_to_point) - _e109);
    float2 straight_border_inner_corner_to_point_1 = (corner_to_point_4 + reduced_border_1);
    bool near_curve = (_e49.signed_distance.segment != FIGMA_SEGMENT_STRAIGHT);
    if ((((straight_border_inner_corner_to_point_1.x < -0.5) && (straight_border_inner_corner_to_point_1.y < -0.5)) && !(near_curve))) {
        const float4 _e127 = blend_color(_e22, _e12);
        return _e127;
    }
    float outer = _e49.signed_distance.distance_;
    if (near_curve) {
        float2 normal_5 = abs(_e49.signed_distance.normal);
        float _e138 = border.x;
        float _e145 = border.y;
        float2 active_sides = float2(((_e138 > 0.0) ? 1.0 : 0.0), ((_e145 > 0.0) ? 1.0 : 0.0));
        float2 _e150 = border;
        float effective_width = (length((_e150 * normal_5)) - (PIXEL_ANTIALIAS_RADIUS * (1.0 - length((active_sides * normal_5)))));
        inner = -((outer + effective_width));
    } else {
        inner = -(max(straight_border_inner_corner_to_point_1.x, straight_border_inner_corner_to_point_1.y));
    }
    color_1 = _e22;
    float _e167 = inner;
    if ((max(_e167, outer) < PIXEL_ANTIALIAS_RADIUS)) {
        const Paint _e173 = Paint_new(quad_13.border_color, quad_13.bounds);
        const PreparedPaint _e179 = PreparedPaint_new(input_1.border_solid, input_1.border_color0_, input_1.border_color1_);
        const float4 _e180 = paint_color(_e173, input_1.position.xy, _e179);
        border_color_1 = _e180;
        if ((quad_13.border_style == BorderStyle_Dashed)) {
            float2 _e187 = border;
            const float _e188 = smoothed_dashed_border_alpha(quad_13, _e41, _e49, prepared_8, input_1.corner_lengths, _e187, straight_border_inner_corner_to_point_1);
            float _e189 = border_color_1.w;
            border_color_1.w = (_e189 * _e188);
        }
        float4 _e191 = border_color_1;
        const float4 _e192 = over(_e22, _e191);
        float _e193 = inner;
        const float _e194 = antialiased_coverage(_e193);
        color_1 = lerp(_e22, _e192, float4(_e194, _e194, _e194, _e194));
    }
    float4 _e197 = color_1;
    const float _e198 = antialiased_coverage(outer);
    const float4 _e200 = blend_color(_e197, (_e198 * _e12));
    return _e200;
}
