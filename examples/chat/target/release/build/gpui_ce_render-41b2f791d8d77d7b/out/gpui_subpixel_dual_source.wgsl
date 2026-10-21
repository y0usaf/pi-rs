enable dual_source_blending;
const PI: f32 = 3.141592653589793;
const HALF_TURN_DEGREES: f32 = 180.0;
const FULL_TURN_DEGREES: f32 = 360.0;
const CSS_GRADIENT_OFFSET_DEGREES: f32 = 90.0;
const PIXEL_ANTIALIAS_RADIUS: f32 = 0.5;
const GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS: f32 = 3.0;
const PATTERN_COMPONENT_SCALE: f32 = 255.0;
const PATTERN_PACKING_RADIX: f32 = 65535.0;
const MIN_NORMALIZED_WEIGHT: f32 = 0.00001;
const MIN_PATH_GRADIENT: f32 = 0.001;
const UNDERLINE_WAVE_FREQUENCY: f32 = 2.0;
const UNDERLINE_WAVE_HEIGHT_RATIO: f32 = 0.8;
const GRADIENT_DITHER_SCALE: f32 = 0.6180339887;
const GRADIENT_DITHER_RGB_STRENGTH: f32 = 2.0 / PATTERN_COMPONENT_SCALE;
const GRADIENT_DITHER_ALPHA_STRENGTH: f32 = 3.0 / PATTERN_COMPONENT_SCALE;
const GRADIENT_DITHER_SEED_A: vec2f = vec2f(12.9898, 78.233);
const GRADIENT_DITHER_SEED_B: vec2f = vec2f(39.3460, 11.135);
const GRADIENT_DITHER_MULTIPLIER_A: f32 = 43758.5453;
const GRADIENT_DITHER_MULTIPLIER_B: f32 = 24634.6345;
const DIAGONAL_STRIPE_ANGLE: f32 = PI / 4.0;
const REC_601_LUMA_WEIGHTS: vec3f = vec3f(0.30, 0.59, 0.11);
const LINEAR_RGB_LUMA_WEIGHTS: vec3f = vec3f(0.2126, 0.7152, 0.0722);
const SRGB_DECODE_CUTOFF: f32 = 0.04045;
const SRGB_ENCODE_CUTOFF: f32 = 0.0031308;
const SRGB_TRANSFER_OFFSET: f32 = 0.055;
const SRGB_TRANSFER_SCALE: f32 = 1.055;
const SRGB_DECODE_EXPONENT: f32 = 2.4;
const SRGB_LINEAR_SCALE: f32 = 12.92;
const ERROR_FUNCTION_LINEAR_COEFFICIENT: f32 = 0.278393;
const ERROR_FUNCTION_QUADRATIC_COEFFICIENT: f32 = 0.230389;
const ERROR_FUNCTION_CUBIC_COEFFICIENT: f32 = 0.000972;
const ERROR_FUNCTION_QUARTIC_COEFFICIENT: f32 = 0.078108;
const DARK_TEXT_CONTRAST_SCALE: f32 = 4.0;
const DARK_TEXT_BRIGHTNESS_CUTOFF: f32 = 0.75;
const LINEAR_SRGB_TO_CONE_RESPONSE: mat3x3f = mat3x3f(vec3f(0.4122214708, 0.2119034982, 0.0883024619), vec3f(0.5363325363, 0.6806995451, 0.2817188376), vec3f(0.0514459929, 0.1073969566, 0.6299787005));
const CONE_RESPONSE_TO_OKLAB: mat3x3f = mat3x3f(vec3f(0.2104542553, 1.9779984951, 0.0259040371), vec3f(0.7936177850, -2.4285922050, 0.7827717662), vec3f(-0.0040720468, 0.4505938995, -0.8086757660));
const OKLAB_TO_CONE_RESPONSE: mat3x3f = mat3x3f(vec3f(1.0, 1.0, 1.0), vec3f(0.3963377774, -0.1055613458, -0.0894841775), vec3f(0.2158037573, -0.0638541728, -1.2914855480));
const CONE_RESPONSE_TO_LINEAR_SRGB: mat3x3f = mat3x3f(vec3f(4.0767416621, -1.2684380046, -0.0041960863), vec3f(-3.3077115913, 2.6097574011, -0.7034186147), vec3f(0.2309699292, -0.3413193965, 1.7076147010));
alias ShaderBool = u32;
const ShaderBool_Disabled: u32 = 0u;
const ShaderBool_Enabled: u32 = 1u;
alias BackgroundTag = u32;
const BackgroundTag_Solid: u32 = 0u;
const BackgroundTag_LinearGradient: u32 = 1u;
const BackgroundTag_PatternSlash: u32 = 2u;
const BackgroundTag_Checkerboard: u32 = 3u;
alias ColorSpace = u32;
const ColorSpace_Srgb: u32 = 0u;
const ColorSpace_Oklab: u32 = 1u;
alias BorderStyle = u32;
const BorderStyle_Solid: u32 = 0u;
const BorderStyle_Dashed: u32 = 1u;
alias SurfaceColorFormat = u32;
const SurfaceColorFormat_Rgba: u32 = 0u;
const SurfaceColorFormat_Yuv: u32 = 1u;
alias BlurCompositeClip = u32;
const BlurCompositeClip_None: u32 = 0u;
const BlurCompositeClip_RoundedBounds: u32 = 1u;
alias DownsampleMode = u32;
const DownsampleMode_Copy: u32 = 0u;
const DownsampleMode_HalfResolution: u32 = 1u;

struct GlobalUniforms {
    viewport_size: vec2f,
    premultiplied_alpha: ShaderBool,
    padding: u32
}

struct FontRasterizationUniforms {
    gamma_ratios: vec4f,
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    uses_blue_green_red_subpixel_order: ShaderBool,
    padding: u32
}
@group(0) @binding(0) var<uniform> GLOBALS: GlobalUniforms;
@group(0) @binding(1) var<uniform> FONT_RASTERIZATION: FontRasterizationUniforms;

struct Bounds {
    origin: vec2f,
    size: vec2f
}

struct ContentMask {
    bounds: Bounds,
    fade_out: Edges
}

fn ContentMask_alpha(content_mask: ContentMask, position: vec2f) -> f32 {
    let bounds = content_mask.bounds;
    let fade = content_mask.fade_out;
    var alpha = 1.0;
    if fade.left > 0.0 {
        alpha *= saturate((position.x - bounds.origin.x) / fade.left);
    }
    if fade.right > 0.0 {
        alpha *= saturate((bounds.origin.x + bounds.size.x - position.x) / fade.right);
    }
    if fade.top > 0.0 {
        alpha *= saturate((position.y - bounds.origin.y) / fade.top);
    }
    if fade.bottom > 0.0 {
        alpha *= saturate((bounds.origin.y + bounds.size.y - position.y) / fade.bottom);
    }
    return alpha;
}

fn Bounds_position(bounds: Bounds, unit_position: vec2f) -> vec2f {
    return bounds.origin + unit_position * bounds.size;
}

fn Bounds__1half_size(bounds: Bounds) -> vec2f {
    return bounds.size / 2.0;
}

fn Bounds_center(bounds: Bounds) -> vec2f {
    return bounds.origin + Bounds__1half_size(bounds);
}

struct Corners {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32
}

fn Corners__1is_zero(corners: Corners) -> bool {
    return corners.top_left == 0.0 && corners.top_right == 0.0 && corners.bottom_right == 0.0 && corners.bottom_left == 0.0;
}

struct Edges {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32
}

fn Edges__1is_zero(edges: Edges) -> bool {
    return edges.top == 0.0 && edges.right == 0.0 && edges.bottom == 0.0 && edges.left == 0.0;
}

struct Hsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32
}

struct LinearColorStop {
    color: Hsla,
    percentage: f32
}

struct Background {
    tag: BackgroundTag,
    color_space: ColorSpace,
    solid: Hsla,
    gradient_angle_or_pattern_height: f32,
    colors: array<LinearColorStop, 2>,
    padding: u32
}

struct AtlasTextureId {
    index: u32,
    kind: u32
}

struct AtlasBounds {
    origin: vec2i,
    size: vec2i
}

struct AtlasTile {
    texture_id: AtlasTextureId,
    tile_id: u32,
    padding: u32,
    bounds: AtlasBounds
}

struct TransformationMatrix {
    rotation_scale: mat2x2f,
    translation: vec2f
}

fn TransformationMatrix__1transform_position(transform: TransformationMatrix, position: vec2f) -> vec2f {
    return transpose(transform.rotation_scale) * position + transform.translation;
}

struct RectangleVertex {
    unit_position: vec2f,
    viewport_position: vec2f,
    clip_position: vec4f
}

struct FullscreenVertex {
    texture_coordinates: vec2f,
    clip_position: vec4f
}

fn is_enabled(value: ShaderBool) -> bool {
    return value == ShaderBool_Enabled;
}

fn transparent() -> vec4f {
    return vec4f(0.0, 0.0, 0.0, 0.0);
}

fn antialiased_coverage(signed_distance: f32) -> f32 {
    return saturate(PIXEL_ANTIALIAS_RADIUS - signed_distance);
}

fn premultiply(color: vec4f, coverage: f32) -> vec4f {
    let alpha = color.w * coverage;
    return vec4f(color.x * alpha, color.y * alpha, color.z * alpha, alpha);
}

fn atlas_texture_coordinates(unit_position: vec2f, tile: AtlasTile, atlas_size: vec2u) -> vec2f {
    let tile_origin = vec2f(f32(tile.bounds.origin.x), f32(tile.bounds.origin.y));
    let tile_size = vec2f(f32(tile.bounds.size.x), f32(tile.bounds.size.y));
    let texture_size = vec2f(f32(atlas_size.x), f32(atlas_size.y));
    return (tile_origin + unit_position * tile_size) / texture_size;
}

fn color_brightness(color: vec3f) -> f32 {
    return dot(color, REC_601_LUMA_WEIGHTS);
}

fn light_on_dark_contrast(enhanced_contrast: f32, color: vec3f) -> f32 {
    let darkness = saturate(DARK_TEXT_CONTRAST_SCALE * (DARK_TEXT_BRIGHTNESS_CUTOFF - color_brightness(color)));
    return enhanced_contrast * darkness;
}

fn apply_contrast_and_gamma_correction(sample: f32, color: vec3f, enhanced_contrast_factor: f32, gamma_ratios: vec4f) -> f32 {
    let enhanced_contrast = light_on_dark_contrast(enhanced_contrast_factor, color);
    return _2apply_alpha_correction_f32(_1enhance_contrast_f32(sample, enhanced_contrast), color_brightness(color), gamma_ratios);
}

fn apply_contrast_and_gamma_correction3(sample: vec3f, color: vec3f, enhanced_contrast_factor: f32, gamma_ratios: vec4f) -> vec3f {
    let contrasted = _1enhance_contrast_vec3f(sample, light_on_dark_contrast(enhanced_contrast_factor, color));
    return _2apply_alpha_correction_vec3f(contrasted, color, gamma_ratios);
}

fn rectangle_corner(vertex_id: u32) -> vec2f {
    return vec2f(f32((vertex_id & 1u)), 0.5 * f32((vertex_id & 2u)));
}

fn viewport_to_clip_position(position: vec2f) -> vec4f {
    let clip_position = position / GLOBALS.viewport_size * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
    return vec4f(clip_position.x, clip_position.y, 0.0, 1.0);
}

fn rectangle_vertex(vertex_id: u32, bounds: Bounds) -> RectangleVertex {
    let unit_position = rectangle_corner(vertex_id);
    let viewport_position = Bounds_position(bounds, unit_position);
    return RectangleVertex(unit_position, viewport_position, viewport_to_clip_position(viewport_position));
}

fn transformed_rectangle_vertex(vertex_id: u32, bounds: Bounds, transform: TransformationMatrix) -> RectangleVertex {
    let unit_position = rectangle_corner(vertex_id);
    let viewport_position = TransformationMatrix__1transform_position(transform, Bounds_position(bounds, unit_position));
    return RectangleVertex(unit_position, viewport_position, viewport_to_clip_position(viewport_position));
}

fn fullscreen_vertex(vertex_id: u32) -> FullscreenVertex {
    let texture_coordinates = vec2f(f32(((vertex_id << 1u) & 2u)), f32((vertex_id & 2u)));
    return FullscreenVertex(texture_coordinates, vec4f(texture_coordinates.x * 2.0 - 1.0, 1.0 - texture_coordinates.y * 2.0, 0.0, 1.0));
}

fn clip_distances(position: vec2f, clip_bounds: Bounds) -> vec4f {
    let distance_from_top_left = position - clip_bounds.origin;
    let distance_from_bottom_right = clip_bounds.origin + clip_bounds.size - position;
    return vec4f(distance_from_top_left.x, distance_from_bottom_right.x, distance_from_top_left.y, distance_from_bottom_right.y);
}

fn is_clipped(distances: vec4f) -> bool {
    return min(min(distances.x, distances.y), min(distances.z, distances.w)) < 0.0;
}

fn unclipped_distances() -> vec4f {
    return vec4f(1.0, 1.0, 1.0, 1.0);
}

fn srgb_to_linear(srgb: vec3f) -> vec3f {
    let cutoff = vec3<bool>(srgb.x < SRGB_DECODE_CUTOFF, srgb.y < SRGB_DECODE_CUTOFF, srgb.z < SRGB_DECODE_CUTOFF);
    let higher = pow((srgb + SRGB_TRANSFER_OFFSET) / SRGB_TRANSFER_SCALE, vec3f(SRGB_DECODE_EXPONENT, SRGB_DECODE_EXPONENT, SRGB_DECODE_EXPONENT));
    return select(higher, srgb / SRGB_LINEAR_SCALE, cutoff);
}

fn linear_to_srgb(linear: vec3f) -> vec3f {
    let cutoff = vec3<bool>(linear.x < SRGB_ENCODE_CUTOFF, linear.y < SRGB_ENCODE_CUTOFF, linear.z < SRGB_ENCODE_CUTOFF);
    let inverse_exponent = 1.0 / SRGB_DECODE_EXPONENT;
    let higher = SRGB_TRANSFER_SCALE * pow(linear, vec3f(inverse_exponent, inverse_exponent, inverse_exponent)) - SRGB_TRANSFER_OFFSET;
    return select(higher, linear * SRGB_LINEAR_SCALE, cutoff);
}

fn linear_to_srgba(color: vec4f) -> vec4f {
    let red_green_blue = linear_to_srgb(color.rgb);
    return vec4f(red_green_blue.x, red_green_blue.y, red_green_blue.z, color.w);
}

fn srgba_to_linear(color: vec4f) -> vec4f {
    let red_green_blue = srgb_to_linear(color.rgb);
    return vec4f(red_green_blue.x, red_green_blue.y, red_green_blue.z, color.w);
}

fn hue_to_unit_rgb(hue: f32) -> vec3f {
    let phases = fract(vec3f(hue, hue + 2.0 / 3.0, hue + 1.0 / 3.0));
    return saturate(abs(phases * 6.0 - 3.0) - 1.0);
}

fn hsla_to_rgba(hsla: Hsla) -> vec4f {
    let chroma = hsla.s * (1.0 - abs(2.0 * hsla.l - 1.0));
    let red_green_blue = (hue_to_unit_rgb(hsla.h) - 0.5) * chroma + hsla.l;
    return vec4f(red_green_blue.x, red_green_blue.y, red_green_blue.z, hsla.a);
}

fn linear_srgb_to_oklab(color: vec4f) -> vec4f {
    let linear_srgb = pow(color.rgb, vec3f(2.2, 2.2, 2.2));
    let cone_response = LINEAR_SRGB_TO_CONE_RESPONSE * linear_srgb;
    let cube_root = vec3f(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0);
    let oklab = CONE_RESPONSE_TO_OKLAB * pow(cone_response, cube_root);
    return vec4f(oklab.x, oklab.y, oklab.z, color.w);
}

fn oklab_to_linear_srgb(color: vec4f) -> vec4f {
    let cone_root = OKLAB_TO_CONE_RESPONSE * color.rgb;
    let cone_response = cone_root * cone_root * cone_root;
    let linear_srgb = CONE_RESPONSE_TO_LINEAR_SRGB * cone_response;
    let srgb = pow(linear_srgb, vec3f(1.0 / 2.2, 1.0 / 2.2, 1.0 / 2.2));
    return vec4f(srgb.x, srgb.y, srgb.z, color.w);
}

fn over(below: vec4f, above: vec4f) -> vec4f {
    let alpha = above.w + below.w * (1.0 - above.w);
    if alpha == 0.0 {
        return transparent();
    }
    let red_green_blue = (above.rgb * above.w + below.rgb * below.w * (1.0 - above.w)) / alpha;
    return vec4f(red_green_blue.x, red_green_blue.y, red_green_blue.z, alpha);
}

fn gaussian(position: f32, standard_deviation: f32) -> f32 {
    let variance = standard_deviation * standard_deviation;
    return exp(-(position * position) / (2.0 * variance)) / (sqrt(2.0 * PI) * standard_deviation);
}

fn approximate_error_function(value: vec2f) -> vec2f {
    let signs = sign(value);
    let absolute_value = abs(value);
    let polynomial = 1.0 + absolute_value * (ERROR_FUNCTION_LINEAR_COEFFICIENT + absolute_value * (ERROR_FUNCTION_QUADRATIC_COEFFICIENT + absolute_value * (ERROR_FUNCTION_CUBIC_COEFFICIENT + absolute_value * ERROR_FUNCTION_QUARTIC_COEFFICIENT)));
    let polynomial_squared = polynomial * polynomial;
    return signs - signs / (polynomial_squared * polynomial_squared);
}

fn integrated_rounded_rectangle_coverage(
    horizontal_position: f32,
    vertical_position: f32,
    standard_deviation: f32,
    corner_radius: f32,
    half_size: vec2f,
) -> f32 {
    let distance_beyond_corner = min(half_size.y - corner_radius - abs(vertical_position), 0.0);
    let curved_extent = half_size.x - corner_radius + sqrt(max(0.0, corner_radius * corner_radius - distance_beyond_corner * distance_beyond_corner));
    let integration_bounds = horizontal_position + vec2f(-curved_extent, curved_extent);
    let normalized_bounds = integration_bounds * (sqrt(0.5) / standard_deviation);
    let integral = 0.5 + 0.5 * approximate_error_function(normalized_bounds);
    return integral.y - integral.x;
}

fn pick_corner_radius(center_to_point: vec2f, radii: Corners) -> f32 {
    let left = select(radii.bottom_left, radii.top_left, center_to_point.y < 0.0);
    let right = select(radii.bottom_right, radii.top_right, center_to_point.y < 0.0);
    return select(right, left, center_to_point.x < 0.0);
}

fn rounded_rectangle_signed_distance_from_corner(corner_center_to_point: vec2f, corner_radius: f32) -> f32 {
    if corner_radius == 0.0 {
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else {
        return length(max(vec2f(0.0, 0.0), corner_center_to_point)) + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y)) - corner_radius;
    }
}

fn rounded_rectangle_signed_distance(point: vec2f, bounds: Bounds, corner_radii: Corners) -> f32 {
    let half_size = Bounds__1half_size(bounds);
    let center_to_point = point - Bounds_center(bounds);
    let corner_radius = pick_corner_radius(center_to_point, corner_radii);
    return rounded_rectangle_signed_distance_from_corner(abs(center_to_point) - half_size + corner_radius, corner_radius);
}

fn gaussian_signed_distance_coverage(distance: f32, standard_deviation: f32) -> f32 {
    let normalized = distance / (sqrt(2.0) * standard_deviation);
    return saturate(0.5 - 0.5 * approximate_error_function(vec2f(normalized, normalized)).x);
}

fn blend_color(color: vec4f, alpha_factor: f32) -> vec4f {
    let alpha = color.w * alpha_factor;
    let multiplier = select(1.0, alpha, is_enabled(GLOBALS.premultiplied_alpha));
    return vec4f(color.x * multiplier, color.y * multiplier, color.z * multiplier, alpha);
}

struct Paint {
    background: Background,
    bounds: Bounds
}

fn Paint_new(background: Background, bounds: Bounds) -> Paint {
    return Paint(background, bounds);
}

struct PreparedPaint {
    solid: vec4f,
    color0: vec4f,
    color1: vec4f
}

fn PreparedPaint_new(solid: vec4f, color0: vec4f, color1: vec4f) -> PreparedPaint {
    return PreparedPaint(solid, color0, color1);
}

fn prepare_paint(paint: Paint) -> PreparedPaint {
    var prepared = PreparedPaint_new(transparent(), transparent(), transparent());
    if paint.background.tag == BackgroundTag_LinearGradient {
        prepared.color0 = hsla_to_rgba(paint.background.colors[0u].color);
        prepared.color1 = hsla_to_rgba(paint.background.colors[1u].color);
        if paint.background.color_space == ColorSpace_Srgb {
            prepared.color0 = linear_to_srgba(prepared.color0);
            prepared.color1 = linear_to_srgba(prepared.color1);
        } else {
            prepared.color0 = linear_srgb_to_oklab(prepared.color0);
            prepared.color1 = linear_srgb_to_oklab(prepared.color1);
        }
    } else {
        prepared.solid = hsla_to_rgba(paint.background.solid);
    }
    return prepared;
}

fn linear_gradient_ratio(paint: Paint, position: vec2f) -> f32 {
    let radians = (paint.background.gradient_angle_or_pattern_height % FULL_TURN_DEGREES - CSS_GRADIENT_OFFSET_DEGREES) * PI / HALF_TURN_DEGREES;
    var direction = vec2f(cos(radians), sin(radians));
    if paint.bounds.size.x > paint.bounds.size.y {
        direction.y *= paint.bounds.size.y / paint.bounds.size.x;
    } else {
        direction.x *= paint.bounds.size.x / paint.bounds.size.y;
    }
    let half_size = Bounds__1half_size(paint.bounds);
    var ratio = dot(position - Bounds_center(paint.bounds), direction) / length(direction);
    if abs(direction.x) > abs(direction.y) {
        ratio = (ratio + half_size.x) / paint.bounds.size.x;
    } else {
        ratio = (ratio + half_size.y) / paint.bounds.size.y;
    }
    let first_stop = paint.background.colors[0u].percentage;
    let last_stop = paint.background.colors[1u].percentage;
    return saturate((ratio - first_stop) / (last_stop - first_stop));
}

fn gradient_dither(position: vec2f) -> vec4f {
    let seed = position * GRADIENT_DITHER_SCALE;
    let noise_a = fract(sin(dot(seed, GRADIENT_DITHER_SEED_A)) * GRADIENT_DITHER_MULTIPLIER_A);
    let noise_b = fract(sin(dot(seed, GRADIENT_DITHER_SEED_B)) * GRADIENT_DITHER_MULTIPLIER_B);
    let triangular_noise = noise_a + noise_b - 1.0;
    return vec4f(triangular_noise * GRADIENT_DITHER_RGB_STRENGTH, triangular_noise * GRADIENT_DITHER_RGB_STRENGTH, triangular_noise * GRADIENT_DITHER_RGB_STRENGTH, triangular_noise * GRADIENT_DITHER_ALPHA_STRENGTH);
}

fn linear_gradient_color(paint: Paint, position: vec2f, prepared: PreparedPaint) -> vec4f {
    let ratio = linear_gradient_ratio(paint, position);
    let ratio4 = vec4f(ratio, ratio, ratio, ratio);
    let interpolated = mix(prepared.color0, prepared.color1, ratio4);
    var color = transparent();
    if paint.background.color_space == ColorSpace_Oklab {
        color = oklab_to_linear_srgb(interpolated);
    } else {
        color = srgba_to_linear(interpolated);
    }
    return color + gradient_dither(position);
}

fn slash_pattern_color(paint: Paint, position: vec2f, solid: vec4f) -> vec4f {
    let encoded = paint.background.gradient_angle_or_pattern_height;
    let pattern_width = (encoded / PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
    let pattern_interval = (encoded % PATTERN_PACKING_RADIX) / PATTERN_COMPONENT_SCALE;
    let pattern_height = pattern_width + pattern_interval;
    let period = pattern_height * sin(DIAGONAL_STRIPE_ANGLE);
    let rotation = mat2x2f(vec2f(cos(DIAGONAL_STRIPE_ANGLE), -sin(DIAGONAL_STRIPE_ANGLE)), vec2f(sin(DIAGONAL_STRIPE_ANGLE), cos(DIAGONAL_STRIPE_ANGLE)));
    let pattern = (rotation * (position - paint.bounds.origin)).x % period;
    let distance = min(pattern, period - pattern) - period * (pattern_width / pattern_height) / 2.0;
    var color = solid;
    color.w *= antialiased_coverage(distance);
    return color;
}

fn checkerboard_color(paint: Paint, position: vec2f, solid: vec4f) -> vec4f {
    let relative = position - paint.bounds.origin;
    let square_size = paint.background.gradient_angle_or_pattern_height;
    let colored = (floor(relative.x / square_size) + floor(relative.y / square_size)) % 2.0;
    var color = solid;
    color.w *= saturate(colored);
    return color;
}

fn paint_color(paint: Paint, position: vec2f, prepared: PreparedPaint) -> vec4f {
    var color = prepared.solid;
    switch paint.background.tag {
        case BackgroundTag_Solid: {
            color = prepared.solid;
        }
        case BackgroundTag_LinearGradient: {
            color = linear_gradient_color(paint, position, prepared);
        }
        case BackgroundTag_PatternSlash: {
            color = slash_pattern_color(paint, position, prepared.solid);
        }
        case BackgroundTag_Checkerboard: {
            color = checkerboard_color(paint, position, prepared.solid);
        }
        default: { }
    }
    return color;
}

fn corner_dash_velocity(first: f32, second: f32) -> f32 {
    if first == 0.0 {
        return second;
    } else if second == 0.0 {
        return first;
    } else {
        return min(first, second);
    }
}

fn signed_modulo(value: f32, modulus: f32) -> f32 {
    return value - modulus * trunc(value / modulus);
}

fn dash_coverage(position: f32, period: f32, dash_length: f32, dash_velocity: f32) -> f32 {
    let half_period = period / 2.0;
    let half_dash_length = dash_length / 2.0;
    let centered = signed_modulo(position + half_period - half_dash_length, period) - half_period;
    let signed_distance = abs(centered) - half_dash_length;
    return antialiased_coverage(signed_distance / dash_velocity);
}

fn quarter_ellipse_signed_distance(point: vec2f, radii: vec2f) -> f32 {
    return (length(point / radii) - 1.0) * (radii.x + radii.y) * -0.5;
}

fn _2apply_alpha_correction_f32(alpha: f32, brightness: f32, gamma_ratios: vec4f) -> f32 {
    let brightness_adjustment = brightness * gamma_ratios.x + gamma_ratios.y;
    let correction = brightness_adjustment * alpha + (brightness * gamma_ratios.z + gamma_ratios.w);
    return alpha + alpha * (1.0 - alpha) * correction;
}

fn _1enhance_contrast_f32(alpha: f32, contrast: f32) -> f32 {
    return alpha * (contrast + 1.0) / (alpha * contrast + 1.0);
}

fn _1enhance_contrast_vec3f(alpha: vec3f, contrast: f32) -> vec3f {
    return alpha * (contrast + 1.0) / (alpha * contrast + 1.0);
}

fn _2apply_alpha_correction_vec3f(alpha: vec3f, brightness: vec3f, gamma_ratios: vec4f) -> vec3f {
    let brightness_adjustment = brightness * gamma_ratios.x + gamma_ratios.y;
    let correction = brightness_adjustment * alpha + (brightness * gamma_ratios.z + gamma_ratios.w);
    return alpha + alpha * (1.0 - alpha) * correction;
}
struct SubpixelSprite {
    order: u32,
    padding: u32,
    bounds: Bounds,
    content_mask: ContentMask,
    color: Hsla,
    tile: AtlasTile,
    transformation: TransformationMatrix
}
@group(1) @binding(0) var<storage, read> SUBPIXEL_SPRITES: array<SubpixelSprite>;
@group(1) @binding(1) var SPRITE_TEXTURE: texture_2d<f32>;
@group(1) @binding(2) var SPRITE_SAMPLER: sampler;

struct SubpixelSpriteOutput {
    @builtin(position) position: vec4f,
    @location(0) tile_position: vec2f,
    @location(1) @interpolate(flat) color: vec4f,
    @location(2) @interpolate(flat) sprite_id: u32,
    @location(3) clip_distances: vec4f
}

struct SubpixelSpriteFragmentOutput {
    @location(0) @blend_src(0) foreground: vec4f,
    @location(0) @blend_src(1) alpha: vec4f
}

@vertex fn vertex_subpixel_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> SubpixelSpriteOutput {
    let sprite = SUBPIXEL_SPRITES[u32(instance_id)];
    let vertex = transformed_rectangle_vertex(vertex_id, sprite.bounds, sprite.transformation);
    return SubpixelSpriteOutput(vertex.clip_position, atlas_texture_coordinates(vertex.unit_position, sprite.tile, textureDimensions(SPRITE_TEXTURE)), hsla_to_rgba(sprite.color), instance_id, clip_distances(vertex.viewport_position, sprite.content_mask.bounds));
}

@fragment fn fragment_subpixel_sprite(input: SubpixelSpriteOutput) -> SubpixelSpriteFragmentOutput {
    if is_clipped(input.clip_distances) {
        return SubpixelSpriteFragmentOutput(transparent(), transparent());
    }
    let sprite = SUBPIXEL_SPRITES[u32(input.sprite_id)];
    let sampled = textureSampleLevel(SPRITE_TEXTURE, SPRITE_SAMPLER, input.tile_position, 0.0).rgb;
    let sample = select(sampled, sampled.bgr, is_enabled(FONT_RASTERIZATION.uses_blue_green_red_subpixel_order));
    let alpha_corrected = apply_contrast_and_gamma_correction3(sample, input.color.rgb, FONT_RASTERIZATION.subpixel_enhanced_contrast, FONT_RASTERIZATION.gamma_ratios);
    let mask_alpha = ContentMask_alpha(sprite.content_mask, input.position.xy);
    return SubpixelSpriteFragmentOutput(vec4f(input.color.x, input.color.y, input.color.z, 1.0), vec4f(input.color.w * alpha_corrected.x * mask_alpha, input.color.w * alpha_corrected.y * mask_alpha, input.color.w * alpha_corrected.z * mask_alpha, 1.0));
}
