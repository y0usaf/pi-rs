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
const SUPERELLIPSE_DIAGONAL_INSET: f32 = 0.2928932188134524;
const FIGMA_SEGMENT_STRAIGHT: u32 = 0u;
const FIGMA_SEGMENT_FIRST_CUBIC: u32 = 1u;
const FIGMA_SEGMENT_ARC: u32 = 2u;
const FIGMA_SEGMENT_SECOND_CUBIC: u32 = 3u;
const FIGMA_NO_CORNER: u32 = 4u;
const FIGMA_EPSILON: f32 = 0.000001;

fn corner_values(corner_radii: Corners) -> vec4f {
    return vec4f(corner_radii.top_left, corner_radii.top_right, corner_radii.bottom_right, corner_radii.bottom_left);
}

fn normalized_superellipse_power(corner_smoothing: f32) -> f32 {
    let smoothing = clamp(corner_smoothing, 0.0, 1.0);
    let normalized_diagonal = 1.0 - SUPERELLIPSE_DIAGONAL_INSET / (1.0 + smoothing);
    return -log(2.0) / log(normalized_diagonal);
}

fn normalized_superellipse_reaches(corner_radii: Corners, corner_smoothing: f32) -> vec4f {
    return max(corner_values(corner_radii), vec4f(0.0, 0.0, 0.0, 0.0)) * (1.0 + clamp(corner_smoothing, 0.0, 1.0));
}

fn can_use_normalized_superellipse(size: vec2f, corner_radii: Corners, corner_smoothing: f32) -> bool {
    let radii = max(corner_values(corner_radii), vec4f(0.0, 0.0, 0.0, 0.0));
    let reaches = normalized_superellipse_reaches(corner_radii, corner_smoothing);
    return corner_smoothing > 0.0 && size.x > 0.0 && size.y > 0.0 && (radii.x > 0.0 || radii.y > 0.0 || radii.z > 0.0 || radii.w > 0.0) && reaches.x + reaches.y <= size.x && reaches.y + reaches.z <= size.y && reaches.z + reaches.w <= size.x && reaches.w + reaches.x <= size.y;
}

fn normalized_superellipse_signed_distance_from_corner(corner_to_point: vec2f, corner_radius: f32, corner_smoothing: f32, power: f32) -> f32 {
    let extent = max(corner_radius, 0.0) * (1.0 + clamp(corner_smoothing, 0.0, 1.0));
    let corner_center_to_point = corner_to_point + extent;
    if extent <= 0.0 || corner_center_to_point.x <= 0.0 || corner_center_to_point.y <= 0.0 {
        return max(corner_to_point.x, corner_to_point.y);
    }
    let normalized = corner_center_to_point / extent;
    let powered = pow(normalized, vec2f(power, power));
    let gradient = power * length(pow(normalized, vec2f(power - 1.0, power - 1.0)));
    return extent * (powered.x + powered.y - 1.0) / max(gradient, FIGMA_EPSILON);
}

fn can_use_compact_corner_selection(size: vec2f, reaches: vec4f) -> bool {
    let half_short_side = 0.5 * min(size.x, size.y);
    return reaches.x <= half_short_side && reaches.y <= half_short_side && reaches.z <= half_short_side && reaches.w <= half_short_side;
}

fn compact_normalized_superellipse_signed_distance(
    point: vec2f,
    bounds: Bounds,
    corner_radii: Corners,
    corner_smoothing: f32,
    power: f32,
) -> f32 {
    let half_size = Bounds__1half_size(bounds);
    let center_to_point = point - Bounds_center(bounds);
    let corner_radius = pick_corner_radius(center_to_point, corner_radii);
    let corner_to_point = abs(center_to_point) - half_size;
    return normalized_superellipse_signed_distance_from_corner(corner_to_point, corner_radius, corner_smoothing, power);
}

fn reach_aware_normalized_superellipse_signed_distance(
    point: vec2f,
    bounds: Bounds,
    corner_radii: Corners,
    corner_smoothing: f32,
    reaches: vec4f,
    power: f32,
) -> f32 {
    let local_point = point - bounds.origin;
    let radii = corner_values(corner_radii);
    var distance = figma_nearest_straight_sample(local_point, bounds.size, reaches, reaches).distance;
    var corner = 0u;
    while corner < 4u {
        if figma_is_corner_candidate(local_point, bounds.size, reaches[corner], reaches[corner], corner) {
            let candidate = normalized_superellipse_signed_distance_from_corner(figma_corner_to_point(local_point, bounds.size, corner), radii[corner], corner_smoothing, power);
            if abs(candidate) <= abs(distance) {
                distance = candidate;
            }
        }
        corner += 1u;
    }
    return distance;
}

fn normalized_superellipse_signed_distance(
    point: vec2f,
    bounds: Bounds,
    corner_radii: Corners,
    corner_smoothing: f32,
    reaches: vec4f,
    power: f32,
) -> f32 {
    if can_use_compact_corner_selection(bounds.size, reaches) {
        return compact_normalized_superellipse_signed_distance(point, bounds, corner_radii, corner_smoothing, power);
    }
    return reach_aware_normalized_superellipse_signed_distance(point, bounds, corner_radii, corner_smoothing, reaches, power);
}

struct FigmaCornerLayout {
    horizontal_budgets: vec4f,
    vertical_budgets: vec4f
}

struct FigmaCornerExtents {
    horizontal: vec4f,
    vertical: vec4f
}

fn figma_split_side(length: f32, first_radius: f32, second_radius: f32) -> vec2f {
    let total_radius = first_radius + second_radius;
    if total_radius == 0.0 {
        return vec2f(0.0, 0.0);
    }
    let first_budget = length * first_radius / total_radius;
    return vec2f(first_budget, length - first_budget);
}

fn figma_corner_layout(size: vec2f, corner_radii: Corners) -> FigmaCornerLayout {
    var radii = array(max(corner_radii.top_left, 0.0), max(corner_radii.top_right, 0.0), max(corner_radii.bottom_right, 0.0), max(corner_radii.bottom_left, 0.0));
    var budgets = array(-1.0, -1.0, -1.0, -1.0);
    var order = array(0u, 1u, 3u, 2u);
    var sort_pass = 0u;
    while sort_pass < 3u {
        var index = 0u;
        while index < 3u - sort_pass {
            if radii[u32(order[u32(index)])] < radii[u32(order[u32((index + 1u))])] {
                let swap = order[u32(index)];
                order[u32(index)] = order[u32((index + 1u))];
                order[u32((index + 1u))] = swap;
            }
            index += 1u;
        }
        sort_pass += 1u;
    }
    var rank = 0u;
    while rank < 4u {
        let corner = order[u32(rank)];
        let radius = radii[u32(corner)];
        var horizontal_neighbor = 0u;
        var vertical_neighbor = 0u;
        switch corner {
            case 0u: {
                horizontal_neighbor = 1u;
                vertical_neighbor = 3u;
            }
            case 1u: {
                horizontal_neighbor = 0u;
                vertical_neighbor = 2u;
            }
            case 2u: {
                horizontal_neighbor = 3u;
                vertical_neighbor = 1u;
            }
            default: {
                horizontal_neighbor = 2u;
                vertical_neighbor = 0u;
            }
        }
        let horizontal_radius = radii[u32(horizontal_neighbor)];
        var horizontal_budget = 0.0;
        if radius != 0.0 || horizontal_radius != 0.0 {
            if budgets[u32(horizontal_neighbor)] >= 0.0 {
                horizontal_budget = size.x - budgets[u32(horizontal_neighbor)];
            } else {
                horizontal_budget = size.x * radius / (radius + horizontal_radius);
            }
        }
        let vertical_radius = radii[u32(vertical_neighbor)];
        var vertical_budget = 0.0;
        if radius != 0.0 || vertical_radius != 0.0 {
            if budgets[u32(vertical_neighbor)] >= 0.0 {
                vertical_budget = size.y - budgets[u32(vertical_neighbor)];
            } else {
                vertical_budget = size.y * radius / (radius + vertical_radius);
            }
        }
        let budget = max(0.0, min(horizontal_budget, vertical_budget));
        budgets[u32(corner)] = budget;
        radii[u32(corner)] = min(radius, budget);
        rank += 1u;
    }
    let top = figma_split_side(size.x, radii[0u], radii[1u]);
    let bottom = figma_split_side(size.x, radii[3u], radii[2u]);
    let left = figma_split_side(size.y, radii[0u], radii[3u]);
    let right = figma_split_side(size.y, radii[1u], radii[2u]);
    return FigmaCornerLayout(vec4f(top.x, top.y, bottom.y, bottom.x), vec4f(left.x, right.x, right.y, left.y));
}

struct FigmaAxisParams {
    a: f32,
    b: f32,
    p: f32
}

struct FigmaCornerParams {
    radius: f32,
    smoothing: f32,
    arc_sweep: f32,
    c: f32,
    d: f32,
    horizontal: FigmaAxisParams,
    vertical: FigmaAxisParams
}

struct CubicClosestPoint {
    point: vec2f,
    tangent: vec2f,
    distance: f32,
    path_t: f32
}

struct SignedDistanceSample {
    distance: f32,
    normal: vec2f,
    path_t: f32,
    segment: u32
}

struct FigmaRectangleSample {
    signed_distance: SignedDistanceSample,
    corner: u32
}

struct PreparedCorners {
    horizontal_reaches: vec4f,
    vertical_reaches: vec4f,
    smoothing_factors: vec4f,
    superellipse_power: f32
}

fn figma_smoothing_factors(corner_smoothing: f32) -> vec4f {
    let smoothing = clamp(corner_smoothing, 0.0, 1.0);
    let arc_sweep = 0.5 * PI * (1.0 - smoothing);
    let beta = (PI / 4.0) * smoothing;
    let join_handle_factor = tan(0.5 * beta);
    return vec4f(smoothing, sin(0.5 * arc_sweep) * sqrt(2.0), join_handle_factor * cos(beta), join_handle_factor * sin(beta));
}

fn figma_corner_params(corner_radius: f32, horizontal_reach: f32, vertical_reach: f32, smoothing_factors: vec4f) -> FigmaCornerParams {
    let safe_horizontal_reach = max(horizontal_reach, 0.0);
    let safe_vertical_reach = max(vertical_reach, 0.0);
    let radius = min(max(corner_radius, 0.0), min(safe_horizontal_reach, safe_vertical_reach));
    let smoothing = select(0.0, smoothing_factors.x, radius > FIGMA_EPSILON);
    let desired_reach = radius * (1.0 + smoothing);
    let arc_sweep = 0.5 * PI * (1.0 - smoothing);
    let arc_delta = smoothing_factors.y * radius;
    let c = smoothing_factors.z * radius;
    let d = smoothing_factors.w * radius;
    let core_length = arc_delta + c + d;
    let ideal_b = max((desired_reach - core_length) / 3.0, 0.0);
    let horizontal_available = max(safe_horizontal_reach - core_length, 0.0);
    let vertical_available = max(safe_vertical_reach - core_length, 0.0);
    let shared_b = min(ideal_b, min(horizontal_available * (5.0 / 6.0), vertical_available * (5.0 / 6.0)));
    return FigmaCornerParams(radius, smoothing, arc_sweep, c, d, FigmaAxisParams(horizontal_available - shared_b, shared_b, safe_horizontal_reach), FigmaAxisParams(vertical_available - shared_b, shared_b, safe_vertical_reach));
}

fn figma_corner_extents(corner_radii: Corners, horizontal_budgets: vec4f, vertical_budgets: vec4f, corner_smoothing: f32) -> FigmaCornerExtents {
    let zero = vec4f(0.0, 0.0, 0.0, 0.0);
    let horizontal = max(horizontal_budgets, zero);
    let vertical = max(vertical_budgets, zero);
    let radii = min(max(corner_values(corner_radii), zero), min(horizontal, vertical));
    let reaches = radii * (1.0 + clamp(corner_smoothing, 0.0, 1.0));
    return FigmaCornerExtents(min(reaches, horizontal), min(reaches, vertical));
}

fn prepare_corners(size: vec2f, radii: Corners, smoothing: f32, allow_superellipse: bool) -> PreparedCorners {
    let radius_values = corner_values(radii);
    var prepared = PreparedCorners(radius_values, radius_values, vec4f(0.0, 1.0, 0.0, 0.0), 0.0);
    if smoothing <= 0.0 {
        return prepared;
    }
    if allow_superellipse && can_use_normalized_superellipse(size, radii, smoothing) {
        let reaches = normalized_superellipse_reaches(radii, smoothing);
        prepared.horizontal_reaches = reaches;
        prepared.vertical_reaches = reaches;
        prepared.superellipse_power = normalized_superellipse_power(smoothing);
        return prepared;
    }
    let corner_layout = figma_corner_layout(size, radii);
    let extents = figma_corner_extents(radii, corner_layout.horizontal_budgets, corner_layout.vertical_budgets, smoothing);
    prepared.horizontal_reaches = extents.horizontal;
    prepared.vertical_reaches = extents.vertical;
    prepared.smoothing_factors = figma_smoothing_factors(smoothing);
    return prepared;
}

fn figma_cubic_point(axis: FigmaAxisParams, c: f32, d: f32, t: f32) -> vec2f {
    let x1 = 3.0 * axis.a;
    let x2 = 3.0 * (axis.b - axis.a);
    let x3 = axis.a - 2.0 * axis.b + c;
    let t2 = t * t;
    return vec2f(t * (x1 + t * (x2 + t * x3)), d * t2 * t);
}

fn figma_cubic_derivative(axis: FigmaAxisParams, c: f32, d: f32, t: f32) -> vec2f {
    let x1 = 3.0 * axis.a;
    let x2 = 3.0 * (axis.b - axis.a);
    let x3 = axis.a - 2.0 * axis.b + c;
    let t2 = t * t;
    return vec2f(x1 + 2.0 * x2 * t + 3.0 * x3 * t2, 3.0 * d * t2);
}

fn figma_cubic_second_derivative(axis: FigmaAxisParams, c: f32, d: f32, t: f32) -> vec2f {
    let x2 = 3.0 * (axis.b - axis.a);
    let x3 = axis.a - 2.0 * axis.b + c;
    return vec2f(2.0 * x2 + 6.0 * x3 * t, 6.0 * d * t);
}

fn closest_figma_cubic(point: vec2f, axis: FigmaAxisParams, c: f32, d: f32) -> CubicClosestPoint {
    let y_seed = pow(clamp(point.y / max(d, 0.00001), 0.0, 1.0), 1.0 / 3.0);
    let chord = figma_cubic_point(axis, c, d, 1.0);
    let chord_seed = clamp(dot(point, chord) / max(dot(chord, chord), FIGMA_EPSILON), 0.0, 1.0);
    let y_delta = figma_cubic_point(axis, c, d, y_seed) - point;
    let chord_delta = figma_cubic_point(axis, c, d, chord_seed) - point;
    var t = select(y_seed, chord_seed, dot(chord_delta, chord_delta) < dot(y_delta, y_delta));
    var iteration = 0u;
    while iteration < 4u {
        let curve_point = figma_cubic_point(axis, c, d, t);
        let tangent = figma_cubic_derivative(axis, c, d, t);
        let second_derivative = figma_cubic_second_derivative(axis, c, d, t);
        let delta = curve_point - point;
        let denominator = dot(tangent, tangent) + dot(delta, second_derivative);
        if abs(denominator) > FIGMA_EPSILON {
            t = clamp(t - dot(delta, tangent) / denominator, 0.0, 1.0);
        }
        iteration += 1u;
    }
    var closest_t = t;
    var closest_point = figma_cubic_point(axis, c, d, t);
    var closest_distance = length(point - closest_point);
    let start = vec2f(0.0, 0.0);
    let start_distance = length(point - start);
    if start_distance < closest_distance {
        closest_t = 0.0;
        closest_point = start;
        closest_distance = start_distance;
    }
    let end_distance = length(point - chord);
    if end_distance < closest_distance {
        closest_t = 1.0;
        closest_point = chord;
        closest_distance = end_distance;
    }
    return CubicClosestPoint(closest_point, figma_cubic_derivative(axis, c, d, closest_t), closest_distance, closest_t);
}

fn figma_signed_distance(delta: vec2f, normal: vec2f, distance: f32) -> f32 {
    return select(-distance, distance, dot(delta, normal) >= 0.0);
}

fn figma_cross_2d(a: vec2f, b: vec2f) -> f32 {
    return a.x * b.y - a.y * b.x;
}

fn figma_unfold_normal(normal: vec2f, mirrored: bool) -> vec2f {
    return select(vec2f(normal.x, -normal.y), vec2f(-normal.y, normal.x), mirrored);
}

fn figma_corner_signed_distance(corner_to_point: vec2f, params: FigmaCornerParams) -> SignedDistanceSample {
    let z = corner_to_point + vec2f(params.horizontal.p, params.vertical.p);
    if params.radius <= FIGMA_EPSILON || z.x <= 0.0 || z.y <= 0.0 {
        return SignedDistanceSample(max(corner_to_point.x, corner_to_point.y), select(vec2f(0.0, 1.0), vec2f(1.0, 0.0), corner_to_point.x > corner_to_point.y), 0.0, FIGMA_SEGMENT_STRAIGHT);
    }
    let horizontal_point = vec2f(z.x, params.vertical.p - z.y);
    let vertical_point = vec2f(z.y, params.horizontal.p - z.x);
    let circle_center = vec2f(params.horizontal.p - params.radius, params.radius);
    let join = vec2f(params.horizontal.a + params.horizontal.b + params.c, params.d);
    let start_direction = (join - circle_center) / params.radius;
    let to_point = horizontal_point - circle_center;
    let to_point_length = length(to_point);
    let point_direction = select(start_direction, to_point / max(to_point_length, FIGMA_EPSILON), to_point_length > FIGMA_EPSILON);
    let arc_angle = clamp(atan2(figma_cross_2d(start_direction, point_direction), dot(start_direction, point_direction)), 0.0, params.arc_sweep);
    let arc_sine = sin(arc_angle);
    let arc_cosine = cos(arc_angle);
    let arc_normal = vec2f(arc_cosine * start_direction.x - arc_sine * start_direction.y, arc_sine * start_direction.x + arc_cosine * start_direction.y);
    let arc_point = circle_center + params.radius * arc_normal;
    let arc_delta = horizontal_point - arc_point;
    let arc_t = select(0.0, arc_angle / max(params.arc_sweep, FIGMA_EPSILON), params.arc_sweep > FIGMA_EPSILON);
    var sample = SignedDistanceSample(figma_signed_distance(arc_delta, arc_normal, length(arc_delta)), figma_unfold_normal(arc_normal, false), arc_t, FIGMA_SEGMENT_ARC);
    if params.smoothing > FIGMA_EPSILON {
        let horizontal_bounds_delta = max(vec2f(0.0, 0.0), max(-horizontal_point, horizontal_point - join));
        if dot(horizontal_bounds_delta, horizontal_bounds_delta) <= sample.distance * sample.distance * 1.000001 + FIGMA_EPSILON {
            let cubic = closest_figma_cubic(horizontal_point, params.horizontal, params.c, params.d);
            let normal = normalize(vec2f(cubic.tangent.y, -cubic.tangent.x));
            let distance = figma_signed_distance(horizontal_point - cubic.point, normal, cubic.distance);
            if abs(distance) <= abs(sample.distance) {
                sample.distance = distance;
                sample.normal = figma_unfold_normal(normal, false);
                sample.path_t = cubic.path_t;
                sample.segment = FIGMA_SEGMENT_FIRST_CUBIC;
            }
        }
        let vertical_join = vec2f(params.vertical.a + params.vertical.b + params.c, params.d);
        let vertical_bounds_delta = max(vec2f(0.0, 0.0), max(-vertical_point, vertical_point - vertical_join));
        if dot(vertical_bounds_delta, vertical_bounds_delta) <= sample.distance * sample.distance * 1.000001 + FIGMA_EPSILON {
            let cubic = closest_figma_cubic(vertical_point, params.vertical, params.c, params.d);
            let normal = normalize(vec2f(cubic.tangent.y, -cubic.tangent.x));
            let distance = figma_signed_distance(vertical_point - cubic.point, normal, cubic.distance);
            if abs(distance) <= abs(sample.distance) {
                sample.distance = distance;
                sample.normal = figma_unfold_normal(normal, true);
                sample.path_t = cubic.path_t;
                sample.segment = FIGMA_SEGMENT_SECOND_CUBIC;
            }
        }
    }
    return sample;
}

fn figma_cubic_length(params: FigmaCornerParams, axis: FigmaAxisParams, end_t: f32) -> f32 {
    let half_t = clamp(end_t, 0.0, 1.0) / 2.0;
    if half_t <= 0.0 || params.smoothing <= 0.0 {
        return 0.0;
    }
    let center = half_t;
    let offset1 = half_t * 0.5384693101;
    let offset2 = half_t * 0.9061798459;
    let speed0 = length(figma_cubic_derivative(axis, params.c, params.d, center));
    let speed1 = length(figma_cubic_derivative(axis, params.c, params.d, center - offset1)) + length(figma_cubic_derivative(axis, params.c, params.d, center + offset1));
    let speed2 = length(figma_cubic_derivative(axis, params.c, params.d, center - offset2)) + length(figma_cubic_derivative(axis, params.c, params.d, center + offset2));
    return half_t * (0.5688888889 * speed0 + 0.4786286705 * speed1 + 0.2369268851 * speed2);
}

fn figma_corner_length(params: FigmaCornerParams) -> f32 {
    return figma_cubic_length(params, params.horizontal, 1.0) + params.radius * params.arc_sweep + figma_cubic_length(params, params.vertical, 1.0);
}

fn figma_corner_progress(params: FigmaCornerParams, sample: SignedDistanceSample, total_length: f32) -> f32 {
    if sample.segment == FIGMA_SEGMENT_FIRST_CUBIC {
        return figma_cubic_length(params, params.horizontal, sample.path_t);
    }
    if sample.segment == FIGMA_SEGMENT_ARC {
        return figma_cubic_length(params, params.horizontal, 1.0) + params.radius * params.arc_sweep * sample.path_t;
    }
    if sample.segment == FIGMA_SEGMENT_SECOND_CUBIC {
        return total_length - figma_cubic_length(params, params.vertical, sample.path_t);
    }
    return 0.0;
}

fn figma_is_corner_candidate(
    point: vec2f,
    size: vec2f,
    horizontal_extent: f32,
    vertical_extent: f32,
    corner: u32,
) -> bool {
    if horizontal_extent <= FIGMA_EPSILON || vertical_extent <= FIGMA_EPSILON {
        return false;
    }
    switch corner {
        case 0u: {
            return point.x <= horizontal_extent && point.y <= vertical_extent;
        }
        case 1u: {
            return size.x - point.x <= horizontal_extent && point.y <= vertical_extent;
        }
        case 2u: {
            return size.x - point.x <= horizontal_extent && size.y - point.y <= vertical_extent;
        }
        default: {
            return point.x <= horizontal_extent && size.y - point.y <= vertical_extent;
        }
    }
}

fn figma_has_corner_candidate(point: vec2f, size: vec2f, horizontal_extents: vec4f, vertical_extents: vec4f) -> bool {
    return figma_is_corner_candidate(point, size, horizontal_extents.x, vertical_extents.x, 0u) || figma_is_corner_candidate(point, size, horizontal_extents.y, vertical_extents.y, 1u) || figma_is_corner_candidate(point, size, horizontal_extents.z, vertical_extents.z, 2u) || figma_is_corner_candidate(point, size, horizontal_extents.w, vertical_extents.w, 3u);
}

fn figma_corner_to_point(point: vec2f, size: vec2f, corner: u32) -> vec2f {
    switch corner {
        case 0u: {
            return -point;
        }
        case 1u: {
            return vec2f(point.x - size.x, -point.y);
        }
        case 2u: {
            return point - size;
        }
        default: {
            return vec2f(-point.x, point.y - size.y);
        }
    }
}

fn figma_orient_corner_normal(normal: vec2f, corner: u32) -> vec2f {
    switch corner {
        case 0u: {
            return -normal;
        }
        case 1u: {
            return vec2f(normal.x, -normal.y);
        }
        case 2u: {
            return normal;
        }
        default: {
            return vec2f(-normal.x, normal.y);
        }
    }
}

fn figma_nearest_straight_sample(point: vec2f, size: vec2f, horizontal_extents: vec4f, vertical_extents: vec4f) -> SignedDistanceSample {
    var nearest_delta = point - vec2f(clamp(point.x, horizontal_extents.x, max(horizontal_extents.x, size.x - horizontal_extents.y)), 0.0);
    var nearest_normal = vec2f(0.0, -1.0);
    var nearest_distance_squared = dot(nearest_delta, nearest_delta);
    var candidate_delta = point - vec2f(size.x, clamp(point.y, vertical_extents.y, max(vertical_extents.y, size.y - vertical_extents.z)));
    var candidate_distance_squared = dot(candidate_delta, candidate_delta);
    if candidate_distance_squared < nearest_distance_squared {
        nearest_delta = candidate_delta;
        nearest_normal = vec2f(1.0, 0.0);
        nearest_distance_squared = candidate_distance_squared;
    }
    candidate_delta = point - vec2f(clamp(point.x, horizontal_extents.w, max(horizontal_extents.w, size.x - horizontal_extents.z)), size.y);
    candidate_distance_squared = dot(candidate_delta, candidate_delta);
    if candidate_distance_squared < nearest_distance_squared {
        nearest_delta = candidate_delta;
        nearest_normal = vec2f(0.0, 1.0);
        nearest_distance_squared = candidate_distance_squared;
    }
    candidate_delta = point - vec2f(0.0, clamp(point.y, vertical_extents.x, max(vertical_extents.x, size.y - vertical_extents.w)));
    candidate_distance_squared = dot(candidate_delta, candidate_delta);
    if candidate_distance_squared < nearest_distance_squared {
        nearest_delta = candidate_delta;
        nearest_normal = vec2f(-1.0, 0.0);
        nearest_distance_squared = candidate_distance_squared;
    }
    return SignedDistanceSample(figma_signed_distance(nearest_delta, nearest_normal, sqrt(nearest_distance_squared)), nearest_normal, 0.0, FIGMA_SEGMENT_STRAIGHT);
}

fn figma_smooth_rectangle_sample(
    point: vec2f,
    bounds: Bounds,
    corner_radii: Corners,
    horizontal_reaches: vec4f,
    vertical_reaches: vec4f,
    smoothing_factors: vec4f,
) -> FigmaRectangleSample {
    let local_point = point - bounds.origin;
    let radii = corner_values(corner_radii);
    var result = FigmaRectangleSample(figma_nearest_straight_sample(local_point, bounds.size, horizontal_reaches, vertical_reaches), FIGMA_NO_CORNER);
    if !figma_has_corner_candidate(local_point, bounds.size, horizontal_reaches, vertical_reaches) {
        return result;
    }
    var corner = 0u;
    while corner < 4u {
        if figma_is_corner_candidate(local_point, bounds.size, horizontal_reaches[corner], vertical_reaches[corner], corner) {
            let params = figma_corner_params(radii[corner], horizontal_reaches[corner], vertical_reaches[corner], smoothing_factors);
            if params.radius > FIGMA_EPSILON {
                var candidate = figma_corner_signed_distance(figma_corner_to_point(local_point, bounds.size, corner), params);
                candidate.normal = figma_orient_corner_normal(candidate.normal, corner);
                if abs(candidate.distance) <= abs(result.signed_distance.distance) {
                    result.signed_distance = candidate;
                    result.corner = corner;
                }
            }
        }
        corner += 1u;
    }
    return result;
}

fn prepared_corner_signed_distance(
    point: vec2f,
    bounds: Bounds,
    corner_radii: Corners,
    corner_smoothing: f32,
    prepared: PreparedCorners,
) -> f32 {
    if prepared.superellipse_power > 0.0 {
        return normalized_superellipse_signed_distance(point, bounds, corner_radii, corner_smoothing, prepared.horizontal_reaches, prepared.superellipse_power);
    }
    if prepared.smoothing_factors.x <= 0.0 {
        return rounded_rectangle_signed_distance(point, bounds, corner_radii);
    }
    return figma_smooth_rectangle_sample(point, bounds, corner_radii, prepared.horizontal_reaches, prepared.vertical_reaches, prepared.smoothing_factors).signed_distance.distance;
}
struct BlurUniforms {
    bounds: Bounds,
    content_mask: Bounds,
    corner_radii: Corners,
    direction: vec2f,
    standard_deviation: f32,
    opacity: f32,
    sample_count: u32,
    sample_step: f32,
    composite_clip: BlurCompositeClip,
    downsample_mode: DownsampleMode,
    source_size: vec2f,
    target_size: vec2f,
    corner_smoothing: f32,
    padding0: u32,
    padding1: u32,
    padding2: u32
}
@group(1) @binding(0) var<uniform> BLUR_LOCALS: BlurUniforms;
@group(1) @binding(1) var BLUR_TEXTURE: texture_2d<f32>;
@group(1) @binding(2) var BLUR_SAMPLER: sampler;

struct BlurCompositeVertexData {
    position: vec4f,
    texture_coordinates: vec2f,
    clip_distances: vec4f
}

fn prepare_blur_composite_vertex(vertex_id: u32) -> BlurCompositeVertexData {
    let vertex = rectangle_vertex(vertex_id, BLUR_LOCALS.bounds);
    return BlurCompositeVertexData(vertex.clip_position, vertex.unit_position, clip_distances(vertex.viewport_position, BLUR_LOCALS.content_mask));
}

fn blur_composite_color(position: vec2f, coverage: f32) -> vec4f {
    let blurred = textureSampleLevel(BLUR_TEXTURE, BLUR_SAMPLER, position / BLUR_LOCALS.target_size, 0.0);
    let factor = coverage * BLUR_LOCALS.opacity;
    return vec4f(blurred.x * factor, blurred.y * factor, blurred.z * factor, blurred.w * factor);
}

struct BlurVarying {
    @builtin(position) position: vec4f,
    @location(0) texture_coordinates: vec2f,
    @location(3) clip_distances: vec4f
}

@vertex fn vertex_blur_fullscreen(@builtin(vertex_index) vertex_id: u32) -> BlurVarying {
    let vertex = fullscreen_vertex(vertex_id);
    return BlurVarying(vertex.clip_position, vertex.texture_coordinates, unclipped_distances());
}

@fragment fn fragment_blur_downsample(input: BlurVarying) -> @location(0) vec4f {
    if BLUR_LOCALS.downsample_mode == DownsampleMode_HalfResolution {
        let destination = floor(input.position.xy);
        let source_coordinates = min((destination + 0.5) / BLUR_LOCALS.target_size, (BLUR_LOCALS.source_size - 0.5) / BLUR_LOCALS.source_size);
        return textureSampleLevel(BLUR_TEXTURE, BLUR_SAMPLER, source_coordinates, 0.0);
    }
    return textureSampleLevel(BLUR_TEXTURE, BLUR_SAMPLER, input.texture_coordinates, 0.0);
}

fn sample_blur_texture(texture_coordinates: vec2f) -> vec4f {
    return textureSampleLevel(BLUR_TEXTURE, BLUR_SAMPLER, texture_coordinates, 0.0);
}

fn gaussian_blur(texture_coordinates: vec2f) -> vec4f {
    let uniforms = BLUR_LOCALS;
    let center_weight = gaussian(0.0, uniforms.standard_deviation);
    var weighted_color = sample_blur_texture(texture_coordinates) * center_weight;
    var weight_sum = center_weight;
    var sample_index = 1u;
    while sample_index <= uniforms.sample_count {
        let distance = f32(sample_index) * uniforms.sample_step;
        let weight = gaussian(distance, uniforms.standard_deviation);
        let coordinate_offset = uniforms.direction * distance;
        let symmetric_pair = sample_blur_texture(texture_coordinates - coordinate_offset) + sample_blur_texture(texture_coordinates + coordinate_offset);
        weighted_color = weighted_color + symmetric_pair * weight;
        weight_sum += 2.0 * weight;
        sample_index += 1u;
    }
    return weighted_color / max(weight_sum, MIN_NORMALIZED_WEIGHT);
}

@fragment fn fragment_blur(input: BlurVarying) -> @location(0) vec4f {
    return gaussian_blur(input.texture_coordinates);
}

@vertex fn vertex_blur_composite(@builtin(vertex_index) vertex_id: u32) -> BlurVarying {
    let vertex = prepare_blur_composite_vertex(vertex_id);
    return BlurVarying(vertex.position, vertex.texture_coordinates, vertex.clip_distances);
}

@fragment fn fragment_blur_composite(input: BlurVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let coverage = select(1.0, antialiased_coverage(rounded_rectangle_signed_distance(input.position.xy, BLUR_LOCALS.bounds, BLUR_LOCALS.corner_radii)), BLUR_LOCALS.composite_clip == BlurCompositeClip_RoundedBounds);
    return blur_composite_color(input.position.xy, coverage);
}

struct SmoothedBlurVarying {
    @builtin(position) position: vec4f,
    @location(0) texture_coordinates: vec2f,
    @location(1) @interpolate(flat) horizontal_corner_reaches: vec4f,
    @location(2) @interpolate(flat) vertical_corner_reaches: vec4f,
    @location(3) clip_distances: vec4f,
    @location(4) @interpolate(flat) smoothing_factors: vec4f,
    @location(5) @interpolate(flat) superellipse_power: f32
}

@vertex fn vertex_smoothed_blur_composite(@builtin(vertex_index) vertex_id: u32) -> SmoothedBlurVarying {
    let vertex = prepare_blur_composite_vertex(vertex_id);
    let prepared = prepare_corners(BLUR_LOCALS.bounds.size, BLUR_LOCALS.corner_radii, BLUR_LOCALS.corner_smoothing, true);
    return SmoothedBlurVarying(vertex.position, vertex.texture_coordinates, prepared.horizontal_reaches, prepared.vertical_reaches, vertex.clip_distances, prepared.smoothing_factors, prepared.superellipse_power);
}

@fragment fn fragment_smoothed_blur_composite(input: SmoothedBlurVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let coverage = select(1.0, antialiased_coverage(prepared_corner_signed_distance(input.position.xy, BLUR_LOCALS.bounds, BLUR_LOCALS.corner_radii, BLUR_LOCALS.corner_smoothing, PreparedCorners(input.horizontal_corner_reaches, input.vertical_corner_reaches, input.smoothing_factors, input.superellipse_power))), BLUR_LOCALS.composite_clip == BlurCompositeClip_RoundedBounds);
    return blur_composite_color(input.position.xy, coverage);
}
struct MonochromeSprite {
    order: u32,
    padding: u32,
    bounds: Bounds,
    content_mask: ContentMask,
    color: Hsla,
    tile: AtlasTile,
    transformation: TransformationMatrix
}
@group(1) @binding(0) var<storage, read> MONOCHROME_SPRITES: array<MonochromeSprite>;
@group(1) @binding(1) var MONOCHROME_TEXTURE: texture_2d<f32>;
@group(1) @binding(2) var MONOCHROME_SAMPLER: sampler;

struct MonochromeSpriteVarying {
    @builtin(position) position: vec4f,
    @location(0) tile_position: vec2f,
    @location(1) @interpolate(flat) color: vec4f,
    @location(2) @interpolate(flat) sprite_id: u32,
    @location(3) clip_distances: vec4f
}

@vertex fn vertex_monochrome_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> MonochromeSpriteVarying {
    let sprite = MONOCHROME_SPRITES[u32(instance_id)];
    let vertex = transformed_rectangle_vertex(vertex_id, sprite.bounds, sprite.transformation);
    return MonochromeSpriteVarying(vertex.clip_position, atlas_texture_coordinates(vertex.unit_position, sprite.tile, textureDimensions(MONOCHROME_TEXTURE)), hsla_to_rgba(sprite.color), instance_id, clip_distances(vertex.viewport_position, sprite.content_mask.bounds));
}

@fragment fn fragment_monochrome_sprite(input: MonochromeSpriteVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let sprite = MONOCHROME_SPRITES[u32(input.sprite_id)];
    let sample = textureSampleLevel(MONOCHROME_TEXTURE, MONOCHROME_SAMPLER, input.tile_position, 0.0).x;
    let corrected = apply_contrast_and_gamma_correction(sample, input.color.rgb, FONT_RASTERIZATION.grayscale_enhanced_contrast, FONT_RASTERIZATION.gamma_ratios);
    return blend_color(input.color, corrected * ContentMask_alpha(sprite.content_mask, input.position.xy));
}
struct PathSprite {
    bounds: Bounds
}
@group(1) @binding(0) var<storage, read> PATH_SPRITES: array<PathSprite>;
@group(1) @binding(1) var PATH_TEXTURE: texture_2d<f32>;
@group(1) @binding(2) var PATH_SAMPLER: sampler;

struct PathVarying {
    @builtin(position) position: vec4f,
    @location(0) texture_coords: vec2f
}

@vertex fn vertex_path(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PathVarying {
    let sprite = PATH_SPRITES[u32(instance_id)];
    let vertex = rectangle_vertex(vertex_id, sprite.bounds);
    return PathVarying(vertex.clip_position, vertex.viewport_position / GLOBALS.viewport_size);
}

@fragment fn fragment_path(input: PathVarying) -> @location(0) vec4f {
    return textureSample(PATH_TEXTURE, PATH_SAMPLER, input.texture_coords);
}
struct PathRasterizationVertex {
    xy_position: vec2f,
    curve_position: vec2f,
    color: Background,
    bounds: Bounds
}
@group(1) @binding(0) var<storage, read> PATH_VERTICES: array<PathRasterizationVertex>;

fn quadratic_bezier_coverage(curve_position: vec2f) -> f32 {
    let horizontal_derivative = dpdx(curve_position);
    let vertical_derivative = dpdy(curve_position);
    let horizontal_gradient = vec2f(horizontal_derivative.x, vertical_derivative.x);
    if length(horizontal_gradient) < MIN_PATH_GRADIENT {
        return 1.0;
    }
    let implicit_gradient = 2.0 * curve_position.xx * horizontal_gradient - vec2f(horizontal_derivative.y, vertical_derivative.y);
    let implicit_value = curve_position.x * curve_position.x - curve_position.y;
    return antialiased_coverage(implicit_value / length(implicit_gradient));
}

struct PathRasterizationVarying {
    @builtin(position) position: vec4f,
    @location(0) curve_position: vec2f,
    @location(1) @interpolate(flat) vertex_id: u32,
    @location(3) clip_distances: vec4f
}

@vertex fn vertex_path_rasterization(@builtin(vertex_index) vertex_id: u32) -> PathRasterizationVarying {
    let vertex = PATH_VERTICES[u32(vertex_id)];
    return PathRasterizationVarying(viewport_to_clip_position(vertex.xy_position), vertex.curve_position, vertex_id, clip_distances(vertex.xy_position, vertex.bounds));
}

@fragment fn fragment_path_rasterization(input: PathRasterizationVarying) -> @location(0) vec4f {
    let coverage = quadratic_bezier_coverage(input.curve_position);
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let vertex = PATH_VERTICES[u32(input.vertex_id)];
    let paint = Paint_new(vertex.color, vertex.bounds);
    let color = paint_color(paint, input.position.xy, prepare_paint(paint));
    return premultiply(color, coverage);
}
struct PolychromeSprite {
    order: u32,
    grayscale: ShaderBool,
    opacity: f32,
    corner_smoothing: f32,
    bounds: Bounds,
    content_mask: ContentMask,
    corner_radii: Corners,
    tile: AtlasTile
}
@group(1) @binding(0) var<storage, read> POLYCHROME_SPRITES: array<PolychromeSprite>;
@group(1) @binding(1) var POLYCHROME_TEXTURE: texture_2d<f32>;
@group(1) @binding(2) var POLYCHROME_SAMPLER: sampler;

struct PolychromeVertexData {
    position: vec4f,
    tile_position: vec2f,
    sprite_id: u32,
    clip_distances: vec4f
}

fn prepare_polychrome_vertex(vertex_id: u32, instance_id: u32, sprite: PolychromeSprite) -> PolychromeVertexData {
    let vertex = rectangle_vertex(vertex_id, sprite.bounds);
    return PolychromeVertexData(vertex.clip_position, atlas_texture_coordinates(vertex.unit_position, sprite.tile, textureDimensions(POLYCHROME_TEXTURE)), instance_id, clip_distances(vertex.viewport_position, sprite.content_mask.bounds));
}

fn polychrome_color(sprite: PolychromeSprite, tile_position: vec2f) -> vec4f {
    let sample = textureSampleLevel(POLYCHROME_TEXTURE, POLYCHROME_SAMPLER, tile_position, 0.0);
    let grayscale = dot(sample.rgb, LINEAR_RGB_LUMA_WEIGHTS);
    return select(sample, vec4f(grayscale, grayscale, grayscale, sample.w), is_enabled(sprite.grayscale));
}

struct PolychromeSpriteVarying {
    @builtin(position) position: vec4f,
    @location(0) tile_position: vec2f,
    @location(1) @interpolate(flat) sprite_id: u32,
    @location(3) clip_distances: vec4f
}

@vertex fn vertex_polychrome_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PolychromeSpriteVarying {
    let sprite = POLYCHROME_SPRITES[u32(instance_id)];
    let vertex = prepare_polychrome_vertex(vertex_id, instance_id, sprite);
    return PolychromeSpriteVarying(vertex.position, vertex.tile_position, vertex.sprite_id, vertex.clip_distances);
}

@fragment fn fragment_polychrome_sprite(input: PolychromeSpriteVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let sprite = POLYCHROME_SPRITES[u32(input.sprite_id)];
    let color = polychrome_color(sprite, input.tile_position);
    return blend_color(color, sprite.opacity * antialiased_coverage(rounded_rectangle_signed_distance(input.position.xy, sprite.bounds, sprite.corner_radii)) * ContentMask_alpha(sprite.content_mask, input.position.xy));
}

struct SmoothedPolychromeSpriteVarying {
    @builtin(position) position: vec4f,
    @location(0) tile_position: vec2f,
    @location(1) @interpolate(flat) sprite_id: u32,
    @location(2) @interpolate(flat) horizontal_corner_reaches: vec4f,
    @location(3) clip_distances: vec4f,
    @location(4) @interpolate(flat) vertical_corner_reaches: vec4f,
    @location(5) @interpolate(flat) smoothing_factors: vec4f,
    @location(6) @interpolate(flat) superellipse_power: f32
}

@vertex fn vertex_smoothed_polychrome_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> SmoothedPolychromeSpriteVarying {
    let sprite = POLYCHROME_SPRITES[u32(instance_id)];
    let vertex = prepare_polychrome_vertex(vertex_id, instance_id, sprite);
    let prepared = prepare_corners(sprite.bounds.size, sprite.corner_radii, sprite.corner_smoothing, true);
    return SmoothedPolychromeSpriteVarying(vertex.position, vertex.tile_position, vertex.sprite_id, prepared.horizontal_reaches, vertex.clip_distances, prepared.vertical_reaches, prepared.smoothing_factors, prepared.superellipse_power);
}

@fragment fn fragment_smoothed_polychrome_sprite(input: SmoothedPolychromeSpriteVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let sprite = POLYCHROME_SPRITES[u32(input.sprite_id)];
    let color = polychrome_color(sprite, input.tile_position);
    return blend_color(color, sprite.opacity * antialiased_coverage(prepared_corner_signed_distance(input.position.xy, sprite.bounds, sprite.corner_radii, sprite.corner_smoothing, PreparedCorners(input.horizontal_corner_reaches, input.vertical_corner_reaches, input.smoothing_factors, input.superellipse_power))) * ContentMask_alpha(sprite.content_mask, input.position.xy));
}
struct Quad {
    order: u32,
    border_style: BorderStyle,
    border_dashed_length: f32,
    border_dashed_gap: f32,
    bounds: Bounds,
    content_mask: ContentMask,
    background: Background,
    border_color: Background,
    corner_radii: Corners,
    border_widths: Edges,
    corner_smoothing: f32,
    padding: u32
}
@group(1) @binding(0) var<storage, read> QUADS: array<Quad>;
const DEFINITELY_OUTSIDE_INNER_BORDER: f32 = -1.0;

struct QuadGeometry {
    point: vec2f,
    center_to_point: vec2f,
    corner_radius: f32,
    corner_center_to_point: vec2f,
    reduced_border: vec2f,
    straight_border_inner_corner_to_point: vec2f,
    near_rounded_corner: bool,
    unrounded: bool
}

struct BorderDistances {
    outer: f32,
    inner: f32
}

struct DashPosition {
    position: f32,
    perimeter: f32,
    velocity: f32
}

struct RoundedDashLayout {
    side_velocities: Edges,
    corner_velocities: Corners,
    right_start: f32,
    bottom_right_start: f32,
    bottom_left_start: f32,
    left_start: f32,
    top_left_start: f32,
    perimeter: f32
}

fn quad_geometry(quad: Quad, position: vec2f) -> QuadGeometry {
    let half_size = Bounds__1half_size(quad.bounds);
    let point = position - quad.bounds.origin;
    let center_to_point = point - half_size;
    let corner_radius = pick_corner_radius(center_to_point, quad.corner_radii);
    let corner_to_point = abs(center_to_point) - half_size;
    let corner_center_to_point = corner_to_point + corner_radius;
    let border = vec2f(select(quad.border_widths.right, quad.border_widths.left, center_to_point.x < 0.0), select(quad.border_widths.bottom, quad.border_widths.top, center_to_point.y < 0.0));
    let reduced_border = vec2f(select(border.x, -PIXEL_ANTIALIAS_RADIUS, border.x == 0.0), select(border.y, -PIXEL_ANTIALIAS_RADIUS, border.y == 0.0));
    return QuadGeometry(point, center_to_point, corner_radius, corner_center_to_point, reduced_border, corner_to_point + reduced_border, corner_center_to_point.x >= 0.0 && corner_center_to_point.y >= 0.0, Corners__1is_zero(quad.corner_radii));
}

fn is_unaffected_background(geometry: QuadGeometry) -> bool {
    return geometry.straight_border_inner_corner_to_point.x < -PIXEL_ANTIALIAS_RADIUS && geometry.straight_border_inner_corner_to_point.y < -PIXEL_ANTIALIAS_RADIUS && !geometry.near_rounded_corner;
}

fn inner_border_signed_distance(geometry: QuadGeometry, outer_signed_distance: f32) -> f32 {
    if geometry.corner_center_to_point.x <= 0.0 || geometry.corner_center_to_point.y <= 0.0 {
        return -max(geometry.straight_border_inner_corner_to_point.x, geometry.straight_border_inner_corner_to_point.y);
    }
    if geometry.straight_border_inner_corner_to_point.x > 0.0 || geometry.straight_border_inner_corner_to_point.y > 0.0 {
        return DEFINITELY_OUTSIDE_INNER_BORDER;
    }
    if geometry.reduced_border.x == geometry.reduced_border.y {
        return -(outer_signed_distance + geometry.reduced_border.x);
    }
    let ellipse_radii = max(vec2f(0.0, 0.0), geometry.corner_radius - geometry.reduced_border);
    return quarter_ellipse_signed_distance(geometry.corner_center_to_point, ellipse_radii);
}

fn border_distances(geometry: QuadGeometry) -> BorderDistances {
    let outer = rounded_rectangle_signed_distance_from_corner(geometry.corner_center_to_point, geometry.corner_radius);
    return BorderDistances(outer, inner_border_signed_distance(geometry, outer));
}

fn dash_period_per_border_width(quad: Quad) -> f32 {
    return max(quad.border_dashed_length, 0.0) + max(quad.border_dashed_gap, 0.0);
}

fn dash_velocity(border_width: f32, period_per_border_width: f32) -> f32 {
    if border_width <= 0.0 || period_per_border_width <= 0.0 {
        return 0.0;
    } else {
        return 1.0 / period_per_border_width / border_width;
    }
}

fn straight_dash_position(quad: Quad, geometry: QuadGeometry) -> DashPosition {
    let horizontal = geometry.corner_center_to_point.x < geometry.corner_center_to_point.y;
    let border_width = select(max(quad.border_widths.right, quad.border_widths.left), max(quad.border_widths.bottom, quad.border_widths.top), horizontal);
    let velocity = dash_velocity(border_width, dash_period_per_border_width(quad));
    return DashPosition(select(geometry.point.y, geometry.point.x, horizontal) * velocity, select(quad.bounds.size.y, quad.bounds.size.x, horizontal) * velocity, velocity);
}

fn side_dash_velocities(border_widths: Edges, period_per_border_width: f32) -> Edges {
    return Edges(dash_velocity(border_widths.top, period_per_border_width), dash_velocity(border_widths.right, period_per_border_width), dash_velocity(border_widths.bottom, period_per_border_width), dash_velocity(border_widths.left, period_per_border_width));
}

fn straight_side_dash_lengths(bounds: Bounds, radii: Corners, velocities: Edges) -> Edges {
    return Edges((bounds.size.x - radii.top_left - radii.top_right) * velocities.top, (bounds.size.y - radii.top_right - radii.bottom_right) * velocities.right, (bounds.size.x - radii.bottom_right - radii.bottom_left) * velocities.bottom, (bounds.size.y - radii.bottom_left - radii.top_left) * velocities.left);
}

fn corner_dash_velocities(side_velocities: Edges) -> Corners {
    return Corners(corner_dash_velocity(side_velocities.top, side_velocities.left), corner_dash_velocity(side_velocities.top, side_velocities.right), corner_dash_velocity(side_velocities.bottom, side_velocities.right), corner_dash_velocity(side_velocities.bottom, side_velocities.left));
}

fn corner_dash_lengths(radii: Corners, velocities: Corners) -> Corners {
    let quarter_turn = PI / 2.0;
    return Corners(radii.top_left * quarter_turn * velocities.top_left, radii.top_right * quarter_turn * velocities.top_right, radii.bottom_right * quarter_turn * velocities.bottom_right, radii.bottom_left * quarter_turn * velocities.bottom_left);
}

fn rounded_dash_layout(quad: Quad) -> RoundedDashLayout {
    let side_velocities = side_dash_velocities(quad.border_widths, dash_period_per_border_width(quad));
    let side_lengths = straight_side_dash_lengths(quad.bounds, quad.corner_radii, side_velocities);
    let corner_velocities = corner_dash_velocities(side_velocities);
    let corner_lengths = corner_dash_lengths(quad.corner_radii, corner_velocities);
    return perimeter_dash_layout(side_velocities, side_lengths, corner_velocities, corner_values(corner_lengths));
}

fn perimeter_dash_layout(side_velocities: Edges, side_lengths: Edges, corner_velocities: Corners, corner_lengths: vec4f) -> RoundedDashLayout {
    let right_start = side_lengths.top + corner_lengths.y;
    let bottom_right_start = right_start + side_lengths.right;
    let bottom_left_start = bottom_right_start + corner_lengths.z + side_lengths.bottom;
    let left_start = bottom_left_start + corner_lengths.w;
    let top_left_start = left_start + side_lengths.left;
    return RoundedDashLayout(side_velocities, corner_velocities, right_start, bottom_right_start, bottom_left_start, left_start, top_left_start, top_left_start + corner_lengths.x);
}

fn smoothed_corner_lengths(quad: Quad, prepared: PreparedCorners) -> vec4f {
    if quad.border_style != BorderStyle_Dashed || quad.corner_smoothing <= 0.0 {
        return corner_values(quad.corner_radii) * (PI / 2.0);
    }
    let radii = corner_values(quad.corner_radii);
    let top_left = figma_corner_length(figma_corner_params(radii.x, prepared.horizontal_reaches.x, prepared.vertical_reaches.x, prepared.smoothing_factors));
    let top_right = figma_corner_length(figma_corner_params(radii.y, prepared.horizontal_reaches.y, prepared.vertical_reaches.y, prepared.smoothing_factors));
    let bottom_right = figma_corner_length(figma_corner_params(radii.z, prepared.horizontal_reaches.z, prepared.vertical_reaches.z, prepared.smoothing_factors));
    let bottom_left = figma_corner_length(figma_corner_params(radii.w, prepared.horizontal_reaches.w, prepared.vertical_reaches.w, prepared.smoothing_factors));
    return vec4f(top_left, top_right, bottom_right, bottom_left);
}

fn smoothed_dash_layout(quad: Quad, prepared: PreparedCorners, corner_lengths: vec4f) -> RoundedDashLayout {
    let side_velocities = side_dash_velocities(quad.border_widths, dash_period_per_border_width(quad));
    let side_lengths = Edges((quad.bounds.size.x - prepared.horizontal_reaches.x - prepared.horizontal_reaches.y) * side_velocities.top, (quad.bounds.size.y - prepared.vertical_reaches.y - prepared.vertical_reaches.z) * side_velocities.right, (quad.bounds.size.x - prepared.horizontal_reaches.z - prepared.horizontal_reaches.w) * side_velocities.bottom, (quad.bounds.size.y - prepared.vertical_reaches.w - prepared.vertical_reaches.x) * side_velocities.left);
    let corner_velocities = corner_dash_velocities(side_velocities);
    return perimeter_dash_layout(side_velocities, side_lengths, corner_velocities, corner_lengths * corner_values(corner_velocities));
}

fn smoothed_dashed_border_alpha(
    quad: Quad,
    geometry: QuadGeometry,
    rectangle_sample: FigmaRectangleSample,
    prepared: PreparedCorners,
    corner_lengths: vec4f,
    border: vec2f,
    straight_border_inner_corner_to_point: vec2f,
) -> f32 {
    let dash_period_per_width = dash_period_per_border_width(quad);
    if dash_period_per_width <= 0.0 {
        return 1.0;
    }
    let dash_layout = smoothed_dash_layout(quad, prepared, corner_lengths);
    var dash_position = 0.0;
    var dash_velocity = 0.0;
    if rectangle_sample.corner != FIGMA_NO_CORNER && rectangle_sample.signed_distance.segment != FIGMA_SEGMENT_STRAIGHT {
        let corner_idx = rectangle_sample.corner;
        let radii = corner_values(quad.corner_radii);
        let corner_length = corner_lengths[corner_idx];
        let params = figma_corner_params(radii[corner_idx], prepared.horizontal_reaches[corner_idx], prepared.vertical_reaches[corner_idx], prepared.smoothing_factors);
        let progress = figma_corner_progress(params, rectangle_sample.signed_distance, corner_length);
        switch corner_idx {
            case 0u: {
                dash_velocity = dash_layout.corner_velocities.top_left;
                dash_position = dash_layout.top_left_start + (corner_length - progress) * dash_velocity;
            }
            case 1u: {
                dash_velocity = dash_layout.corner_velocities.top_right;
                dash_position = dash_layout.right_start + progress * dash_velocity;
            }
            case 2u: {
                dash_velocity = dash_layout.corner_velocities.bottom_right;
                dash_position = dash_layout.bottom_right_start + (corner_length - progress) * dash_velocity;
            }
            default: {
                dash_velocity = dash_layout.corner_velocities.bottom_left;
                dash_position = dash_layout.bottom_left_start + progress * dash_velocity;
            }
        }
    } else {
        var horizontal = straight_border_inner_corner_to_point.x < straight_border_inner_corner_to_point.y;
        var straight_width = select(border.x, border.y, horizontal);
        if straight_width <= 0.0 {
            horizontal = !horizontal;
            straight_width = select(border.x, border.y, horizontal);
        }
        if horizontal {
            if geometry.center_to_point.y < 0.0 {
                dash_velocity = dash_layout.side_velocities.top;
                dash_position = (geometry.point.x - prepared.horizontal_reaches.x) * dash_velocity;
            } else {
                dash_velocity = dash_layout.side_velocities.bottom;
                dash_position = dash_layout.bottom_left_start - (geometry.point.x - prepared.horizontal_reaches.w) * dash_velocity;
            }
        } else if geometry.center_to_point.x < 0.0 {
            dash_velocity = dash_layout.side_velocities.left;
            dash_position = dash_layout.top_left_start - (geometry.point.y - prepared.vertical_reaches.x) * dash_velocity;
        } else {
            dash_velocity = dash_layout.side_velocities.right;
            dash_position = dash_layout.right_start + (geometry.point.y - prepared.vertical_reaches.y) * dash_velocity;
        }
    }
    let dash_length = max(quad.border_dashed_length, 0.0) / dash_period_per_width;
    if dash_layout.perimeter >= 1.0 {
        let period = dash_layout.perimeter / floor(dash_layout.perimeter);
        return dash_coverage(dash_position, period, dash_length, dash_velocity);
    } else {
        return 1.0;
    }
}

fn rounded_dash_position(quad: Quad, geometry: QuadGeometry) -> DashPosition {
    let radii = quad.corner_radii;
    let dash_layout = rounded_dash_layout(quad);
    let horizontal = geometry.corner_center_to_point.x < geometry.corner_center_to_point.y;
    let on_right = geometry.center_to_point.x >= 0.0;
    let on_bottom = geometry.center_to_point.y >= 0.0;
    let top_position = (geometry.point.x - radii.top_left) * dash_layout.side_velocities.top;
    let right_position = dash_layout.right_start + (geometry.point.y - radii.top_right) * dash_layout.side_velocities.right;
    let bottom_position = dash_layout.bottom_left_start - (geometry.point.x - radii.bottom_left) * dash_layout.side_velocities.bottom;
    let left_position = dash_layout.top_left_start - (geometry.point.y - radii.top_left) * dash_layout.side_velocities.left;
    let horizontal_position = select(top_position, bottom_position, on_bottom);
    let vertical_position = select(left_position, right_position, on_right);
    let side_position = select(vertical_position, horizontal_position, horizontal);
    let horizontal_velocity = select(dash_layout.side_velocities.top, dash_layout.side_velocities.bottom, on_bottom);
    let vertical_velocity = select(dash_layout.side_velocities.left, dash_layout.side_velocities.right, on_right);
    let side_velocity = select(vertical_velocity, horizontal_velocity, horizontal);
    if geometry.near_rounded_corner {
        let corner_position = atan2(geometry.corner_center_to_point.y, geometry.corner_center_to_point.x) * geometry.corner_radius;
        let top_right_position = dash_layout.right_start - corner_position * dash_layout.corner_velocities.top_right;
        let bottom_right_position = dash_layout.bottom_right_start + corner_position * dash_layout.corner_velocities.bottom_right;
        let bottom_left_position = dash_layout.left_start - corner_position * dash_layout.corner_velocities.bottom_left;
        let top_left_position = dash_layout.top_left_start + corner_position * dash_layout.corner_velocities.top_left;
        let right_position = select(top_right_position, bottom_right_position, on_bottom);
        let left_position = select(top_left_position, bottom_left_position, on_bottom);
        let position = select(left_position, right_position, on_right);
        let right_velocity = select(dash_layout.corner_velocities.top_right, dash_layout.corner_velocities.bottom_right, on_bottom);
        let left_velocity = select(dash_layout.corner_velocities.top_left, dash_layout.corner_velocities.bottom_left, on_bottom);
        return DashPosition(position, dash_layout.perimeter, select(left_velocity, right_velocity, on_right));
    }
    return DashPosition(side_position, dash_layout.perimeter, side_velocity);
}

fn dashed_border_alpha(quad: Quad, geometry: QuadGeometry) -> f32 {
    let dash_period_per_width = dash_period_per_border_width(quad);
    if dash_period_per_width <= 0.0 {
        return 1.0;
    }
    var dash = DashPosition(0.0, 0.0, 0.0);
    if geometry.unrounded {
        dash = straight_dash_position(quad, geometry);
    } else {
        dash = rounded_dash_position(quad, geometry);
    }
    let dash_length = max(quad.border_dashed_length, 0.0) / dash_period_per_width;
    let perimeter = dash.perimeter - select(0.0, dash_length, geometry.unrounded);
    if perimeter >= 1.0 {
        let period = perimeter / floor(perimeter);
        return dash_coverage(dash.position, period, dash_length, dash.velocity);
    } else if geometry.unrounded && perimeter > dash_length {
        return dash_coverage(dash.position, perimeter, dash_length, dash.velocity);
    } else {
        return 1.0;
    }
}

struct QuadVertexData {
    position: vec4f,
    border: PreparedPaint,
    quad_id: u32,
    clip_distances: vec4f,
    fill: PreparedPaint
}

fn prepare_quad_vertex(vertex_id: u32, instance_id: u32, quad: Quad) -> QuadVertexData {
    let vertex = rectangle_vertex(vertex_id, quad.bounds);
    return QuadVertexData(vertex.clip_position, prepare_paint(Paint_new(quad.border_color, quad.bounds)), instance_id, clip_distances(vertex.viewport_position, quad.content_mask.bounds), prepare_paint(Paint_new(quad.background, quad.bounds)));
}

struct QuadVarying {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) border_solid: vec4f,
    @location(1) @interpolate(flat) quad_id: u32,
    @location(2) clip_distances: vec4f,
    @location(3) @interpolate(flat) fill_solid: vec4f,
    @location(4) @interpolate(flat) fill_color0: vec4f,
    @location(5) @interpolate(flat) fill_color1: vec4f,
    @location(6) @interpolate(flat) border_color0: vec4f,
    @location(7) @interpolate(flat) border_color1: vec4f
}

@vertex fn vertex_quad(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> QuadVarying {
    let quad = QUADS[u32(instance_id)];
    let vertex = prepare_quad_vertex(vertex_id, instance_id, quad);
    return QuadVarying(vertex.position, vertex.border.solid, vertex.quad_id, vertex.clip_distances, vertex.fill.solid, vertex.fill.color0, vertex.fill.color1, vertex.border.color0, vertex.border.color1);
}

@fragment fn fragment_quad(input: QuadVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let quad = QUADS[u32(input.quad_id)];
    let fade = ContentMask_alpha(quad.content_mask, input.position.xy);
    let fill_color = paint_color(Paint_new(quad.background, quad.bounds), input.position.xy, PreparedPaint_new(input.fill_solid, input.fill_color0, input.fill_color1));
    if Edges__1is_zero(quad.border_widths) && Corners__1is_zero(quad.corner_radii) {
        return blend_color(fill_color, fade);
    }
    let geometry = quad_geometry(quad, input.position.xy);
    if is_unaffected_background(geometry) {
        return blend_color(fill_color, fade);
    }
    let distances = border_distances(geometry);
    var color = fill_color;
    if max(distances.inner, distances.outer) < PIXEL_ANTIALIAS_RADIUS {
        var border_color = paint_color(Paint_new(quad.border_color, quad.bounds), input.position.xy, PreparedPaint_new(input.border_solid, input.border_color0, input.border_color1));
        if quad.border_style == BorderStyle_Dashed {
            border_color.w *= dashed_border_alpha(quad, geometry);
        }
        let blended_border = over(fill_color, border_color);
        let factor = antialiased_coverage(distances.inner);
        color = mix(fill_color, blended_border, vec4f(factor, factor, factor, factor));
    }
    return blend_color(color, antialiased_coverage(distances.outer) * fade);
}

struct SmoothedQuadVarying {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) border_solid: vec4f,
    @location(1) @interpolate(flat) quad_id: u32,
    @location(2) clip_distances: vec4f,
    @location(3) @interpolate(flat) fill_solid: vec4f,
    @location(4) @interpolate(flat) fill_color0: vec4f,
    @location(5) @interpolate(flat) fill_color1: vec4f,
    @location(6) @interpolate(flat) horizontal_corner_reaches: vec4f,
    @location(7) @interpolate(flat) vertical_corner_reaches: vec4f,
    @location(8) @interpolate(flat) corner_lengths: vec4f,
    @location(9) @interpolate(flat) smoothing_factors: vec4f,
    @location(10) @interpolate(flat) superellipse_power: f32,
    @location(11) @interpolate(flat) border_color0: vec4f,
    @location(12) @interpolate(flat) border_color1: vec4f
}

@vertex fn vertex_smoothed_quad(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> SmoothedQuadVarying {
    let quad = QUADS[u32(instance_id)];
    let vertex = prepare_quad_vertex(vertex_id, instance_id, quad);
    let prepared = prepare_corners(quad.bounds.size, quad.corner_radii, quad.corner_smoothing, quad.border_style == BorderStyle_Solid && Edges__1is_zero(quad.border_widths));
    return SmoothedQuadVarying(vertex.position, vertex.border.solid, vertex.quad_id, vertex.clip_distances, vertex.fill.solid, vertex.fill.color0, vertex.fill.color1, prepared.horizontal_reaches, prepared.vertical_reaches, smoothed_corner_lengths(quad, prepared), prepared.smoothing_factors, prepared.superellipse_power, vertex.border.color0, vertex.border.color1);
}

@fragment fn fragment_smoothed_quad(input: SmoothedQuadVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let quad = QUADS[u32(input.quad_id)];
    let fade = ContentMask_alpha(quad.content_mask, input.position.xy);
    let fill_color = paint_color(Paint_new(quad.background, quad.bounds), input.position.xy, PreparedPaint_new(input.fill_solid, input.fill_color0, input.fill_color1));
    let prepared = PreparedCorners(input.horizontal_corner_reaches, input.vertical_corner_reaches, input.smoothing_factors, input.superellipse_power);
    if Edges__1is_zero(quad.border_widths) {
        let distance = prepared_corner_signed_distance(input.position.xy, quad.bounds, quad.corner_radii, quad.corner_smoothing, prepared);
        return blend_color(fill_color, antialiased_coverage(distance) * fade);
    }
    let geometry = quad_geometry(quad, input.position.xy);
    let rectangle_sample = figma_smooth_rectangle_sample(input.position.xy, quad.bounds, quad.corner_radii, prepared.horizontal_reaches, prepared.vertical_reaches, prepared.smoothing_factors);
    var border = vec2f(select(quad.border_widths.right, quad.border_widths.left, geometry.center_to_point.x < 0.0), select(quad.border_widths.bottom, quad.border_widths.top, geometry.center_to_point.y < 0.0));
    switch rectangle_sample.corner {
        case 0u: {
            border = vec2f(quad.border_widths.left, quad.border_widths.top);
        }
        case 1u: {
            border = vec2f(quad.border_widths.right, quad.border_widths.top);
        }
        case 2u: {
            border = vec2f(quad.border_widths.right, quad.border_widths.bottom);
        }
        case 3u: {
            border = vec2f(quad.border_widths.left, quad.border_widths.bottom);
        }
        default: {
        }
    }
    let reduced_border = vec2f(select(border.x, -PIXEL_ANTIALIAS_RADIUS, border.x == 0.0), select(border.y, -PIXEL_ANTIALIAS_RADIUS, border.y == 0.0));
    let half_size = Bounds__1half_size(quad.bounds);
    let corner_to_point = abs(geometry.center_to_point) - half_size;
    let straight_border_inner_corner_to_point = corner_to_point + reduced_border;
    let near_curve = rectangle_sample.signed_distance.segment != FIGMA_SEGMENT_STRAIGHT;
    if straight_border_inner_corner_to_point.x < -PIXEL_ANTIALIAS_RADIUS && straight_border_inner_corner_to_point.y < -PIXEL_ANTIALIAS_RADIUS && !near_curve {
        return blend_color(fill_color, fade);
    }
    let outer = rectangle_sample.signed_distance.distance;
    var inner = 0.0;
    if near_curve {
        let normal = abs(rectangle_sample.signed_distance.normal);
        let active_sides = vec2f(select(0.0, 1.0, border.x > 0.0), select(0.0, 1.0, border.y > 0.0));
        let effective_width = length(border * normal) - PIXEL_ANTIALIAS_RADIUS * (1.0 - length(active_sides * normal));
        inner = -(outer + effective_width);
    } else {
        inner = -max(straight_border_inner_corner_to_point.x, straight_border_inner_corner_to_point.y);
    }
    var color = fill_color;
    if max(inner, outer) < PIXEL_ANTIALIAS_RADIUS {
        var border_color = paint_color(Paint_new(quad.border_color, quad.bounds), input.position.xy, PreparedPaint_new(input.border_solid, input.border_color0, input.border_color1));
        if quad.border_style == BorderStyle_Dashed {
            border_color.w *= smoothed_dashed_border_alpha(quad, geometry, rectangle_sample, prepared, input.corner_lengths, border, straight_border_inner_corner_to_point);
        }
        let blended_border = over(fill_color, border_color);
        let factor = antialiased_coverage(inner);
        color = mix(fill_color, blended_border, vec4f(factor, factor, factor, factor));
    }
    return blend_color(color, antialiased_coverage(outer) * fade);
}
struct Shadow {
    order: u32,
    blur_radius: f32,
    bounds: Bounds,
    corner_radii: Corners,
    content_mask: ContentMask,
    color: Background,
    element_bounds: Bounds,
    element_corner_radii: Corners,
    inset: ShaderBool,
    corner_smoothing: f32
}
@group(1) @binding(0) var<storage, read> SHADOWS: array<Shadow>;
const SHADOW_INTEGRATION_SAMPLE_COUNT: i32 = 4;

fn shadow_geometry(shadow: Shadow) -> Bounds {
    if is_enabled(shadow.inset) {
        return shadow.element_bounds;
    }
    let margin = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow.blur_radius;
    return Bounds(shadow.bounds.origin - vec2f(margin, margin), shadow.bounds.size + vec2f(2.0 * margin, 2.0 * margin));
}

fn shadow_paint(shadow: Shadow) -> Paint {
    var bounds = shadow.bounds;
    if is_enabled(shadow.inset) {
        bounds = shadow.element_bounds;
    }
    return Paint_new(shadow.color, bounds);
}

fn blurred_shadow_coverage(shadow: Shadow, position: vec2f) -> f32 {
    let half_size = Bounds__1half_size(shadow.bounds);
    let center_to_point = position - Bounds_center(shadow.bounds);
    let corner_radius = pick_corner_radius(center_to_point, shadow.corner_radii);
    let vertical_minimum = center_to_point.y - half_size.y;
    let vertical_maximum = center_to_point.y + half_size.y;
    let integration_radius = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow.blur_radius;
    let integration_start = clamp(-integration_radius, vertical_minimum, vertical_maximum);
    let integration_end = clamp(integration_radius, vertical_minimum, vertical_maximum);
    let sample_step = (integration_end - integration_start) / f32(SHADOW_INTEGRATION_SAMPLE_COUNT);
    var coverage = 0.0;
    var sample_index = 0;
    var sample_position = integration_start + sample_step * 0.5;
    while sample_index < SHADOW_INTEGRATION_SAMPLE_COUNT {
        coverage += integrated_rounded_rectangle_coverage(center_to_point.x, center_to_point.y - sample_position, shadow.blur_radius, corner_radius, half_size) * gaussian(sample_position, shadow.blur_radius) * sample_step;
        sample_position += sample_step;
        sample_index += 1;
    }
    return coverage;
}

fn shadow_coverage(shadow: Shadow, position: vec2f) -> f32 {
    var coverage = 0.0;
    if shadow.blur_radius == 0.0 {
        coverage = antialiased_coverage(rounded_rectangle_signed_distance(position, shadow.bounds, shadow.corner_radii));
    } else {
        coverage = blurred_shadow_coverage(shadow, position);
    }
    if is_enabled(shadow.inset) {
        coverage = (1.0 - coverage) * antialiased_coverage(rounded_rectangle_signed_distance(position, shadow.element_bounds, shadow.element_corner_radii));
    }
    return coverage;
}

fn smoothed_shadow_coverage(shadow: Shadow, position: vec2f, prepared: PreparedCorners, prepared_element: PreparedCorners) -> f32 {
    var coverage = 0.0;
    if shadow.blur_radius == 0.0 {
        coverage = antialiased_coverage(prepared_corner_signed_distance(position, shadow.bounds, shadow.corner_radii, shadow.corner_smoothing, prepared));
    } else if !Corners__1is_zero(shadow.corner_radii) {
        let blur_limit = GAUSSIAN_CUTOFF_STANDARD_DEVIATIONS * shadow.blur_radius;
        let half_size = Bounds__1half_size(shadow.bounds);
        let center_to_point = position - Bounds_center(shadow.bounds);
        let box_delta = abs(center_to_point) - half_size;
        let outside_delta = max(box_delta, vec2f(0.0, 0.0));
        if dot(outside_delta, outside_delta) <= blur_limit * blur_limit {
            let local_point = position - shadow.bounds.origin;
            let edge_depth = min(min(local_point.x, shadow.bounds.size.x - local_point.x), min(local_point.y, shadow.bounds.size.y - local_point.y));
            let corner_candidate = figma_has_corner_candidate(local_point, shadow.bounds.size, prepared.horizontal_reaches, prepared.vertical_reaches);
            if !corner_candidate && edge_depth >= blur_limit {
                coverage = 1.0;
            } else {
                let distance = prepared_corner_signed_distance(position, shadow.bounds, shadow.corner_radii, shadow.corner_smoothing, prepared);
                coverage = gaussian_signed_distance_coverage(distance, shadow.blur_radius);
            }
        }
    } else {
        coverage = blurred_shadow_coverage(shadow, position);
    }
    if is_enabled(shadow.inset) {
        coverage = (1.0 - coverage) * antialiased_coverage(prepared_corner_signed_distance(position, shadow.element_bounds, shadow.element_corner_radii, shadow.corner_smoothing, prepared_element));
    }
    return coverage;
}

struct ShadowVertexData {
    position: vec4f,
    paint: PreparedPaint,
    shadow_id: u32,
    clip_distances: vec4f
}

fn prepare_shadow_vertex(vertex_id: u32, instance_id: u32, shadow: Shadow) -> ShadowVertexData {
    let vertex = rectangle_vertex(vertex_id, shadow_geometry(shadow));
    return ShadowVertexData(vertex.clip_position, prepare_paint(shadow_paint(shadow)), instance_id, clip_distances(vertex.viewport_position, shadow.content_mask.bounds));
}

struct ShadowVarying {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) paint_solid: vec4f,
    @location(1) @interpolate(flat) paint_color0: vec4f,
    @location(2) @interpolate(flat) paint_color1: vec4f,
    @location(3) @interpolate(flat) shadow_id: u32,
    @location(4) clip_distances: vec4f
}

@vertex fn vertex_shadow(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> ShadowVarying {
    let shadow = SHADOWS[u32(instance_id)];
    let vertex = prepare_shadow_vertex(vertex_id, instance_id, shadow);
    return ShadowVarying(vertex.position, vertex.paint.solid, vertex.paint.color0, vertex.paint.color1, vertex.shadow_id, vertex.clip_distances);
}

@fragment fn fragment_shadow(input: ShadowVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let shadow = SHADOWS[u32(input.shadow_id)];
    let color = paint_color(shadow_paint(shadow), input.position.xy, PreparedPaint_new(input.paint_solid, input.paint_color0, input.paint_color1));
    return blend_color(color, shadow_coverage(shadow, input.position.xy) * ContentMask_alpha(shadow.content_mask, input.position.xy));
}

struct SmoothedShadowVarying {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) paint_solid: vec4f,
    @location(1) @interpolate(flat) paint_color0: vec4f,
    @location(2) @interpolate(flat) paint_color1: vec4f,
    @location(3) @interpolate(flat) shadow_id: u32,
    @location(4) clip_distances: vec4f,
    @location(5) @interpolate(flat) horizontal_corner_reaches: vec4f,
    @location(6) @interpolate(flat) vertical_corner_reaches: vec4f,
    @location(7) @interpolate(flat) element_horizontal_corner_reaches: vec4f,
    @location(8) @interpolate(flat) element_vertical_corner_reaches: vec4f,
    @location(9) @interpolate(flat) smoothing_factors: vec4f
}

@vertex fn vertex_smoothed_shadow(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> SmoothedShadowVarying {
    let shadow = SHADOWS[u32(instance_id)];
    let vertex = prepare_shadow_vertex(vertex_id, instance_id, shadow);
    let prepared = prepare_corners(shadow.bounds.size, shadow.corner_radii, shadow.corner_smoothing, false);
    let prepared_element = prepare_corners(shadow.element_bounds.size, shadow.element_corner_radii, shadow.corner_smoothing, false);
    return SmoothedShadowVarying(vertex.position, vertex.paint.solid, vertex.paint.color0, vertex.paint.color1, vertex.shadow_id, vertex.clip_distances, prepared.horizontal_reaches, prepared.vertical_reaches, prepared_element.horizontal_reaches, prepared_element.vertical_reaches, prepared.smoothing_factors);
}

@fragment fn fragment_smoothed_shadow(input: SmoothedShadowVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let shadow = SHADOWS[u32(input.shadow_id)];
    let coverage = smoothed_shadow_coverage(shadow, input.position.xy, PreparedCorners(input.horizontal_corner_reaches, input.vertical_corner_reaches, input.smoothing_factors, 0.0), PreparedCorners(input.element_horizontal_corner_reaches, input.element_vertical_corner_reaches, input.smoothing_factors, 0.0));
    let color = paint_color(shadow_paint(shadow), input.position.xy, PreparedPaint_new(input.paint_solid, input.paint_color0, input.paint_color1));
    return blend_color(color, coverage * ContentMask_alpha(shadow.content_mask, input.position.xy));
}
struct SurfaceUniforms {
    bounds: Bounds,
    content_mask: ContentMask,
    color_format: SurfaceColorFormat,
    opacity: f32,
    padding0: u32,
    padding1: u32,
    padding2: u32,
    padding3: u32,
    padding4: u32,
    padding5: u32
}
@group(1) @binding(0) var<uniform> SURFACE_LOCALS: SurfaceUniforms;
@group(1) @binding(1) var SURFACE_TEXTURE: texture_2d<f32>;
@group(1) @binding(2) var SURFACE_CHROMA_TEXTURE: texture_2d<f32>;
@group(1) @binding(3) var SURFACE_SAMPLER: sampler;
const YCBCR_TO_LINEAR_RGB: mat4x4f = mat4x4f(vec4f(1.0000, 1.0000, 1.0000, 0.0), vec4f(0.0000, -0.3441, 1.7720, 0.0), vec4f(1.4020, -0.7141, 0.0000, 0.0), vec4f(-0.7010, 0.5291, -0.8860, 1.0));

fn sample_yuv_surface(texture_position: vec2f) -> vec4f {
    let luma = textureSampleLevel(SURFACE_TEXTURE, SURFACE_SAMPLER, texture_position, 0.0).x;
    let chroma = textureSampleLevel(SURFACE_CHROMA_TEXTURE, SURFACE_SAMPLER, texture_position, 0.0).xy;
    return YCBCR_TO_LINEAR_RGB * vec4f(luma, chroma.x, chroma.y, 1.0);
}

struct SurfaceVarying {
    @builtin(position) position: vec4f,
    @location(0) texture_position: vec2f,
    @location(3) clip_distances: vec4f
}

@vertex fn vertex_surface(@builtin(vertex_index) vertex_id: u32) -> SurfaceVarying {
    let vertex = rectangle_vertex(vertex_id, SURFACE_LOCALS.bounds);
    return SurfaceVarying(vertex.clip_position, vertex.unit_position, clip_distances(vertex.viewport_position, SURFACE_LOCALS.content_mask.bounds));
}

@fragment fn fragment_surface(input: SurfaceVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let locals = SURFACE_LOCALS;
    let fade = ContentMask_alpha(locals.content_mask, input.position.xy);
    if locals.color_format == SurfaceColorFormat_Yuv {
        return sample_yuv_surface(input.texture_position) * locals.opacity * fade;
    }
    return textureSampleLevel(SURFACE_TEXTURE, SURFACE_SAMPLER, input.texture_position, 0.0) * locals.opacity * fade;
}
struct Underline {
    order: u32,
    padding: u32,
    bounds: Bounds,
    content_mask: ContentMask,
    color: Hsla,
    thickness: f32,
    wavy: ShaderBool
}
@group(1) @binding(0) var<storage, read> UNDERLINES: array<Underline>;

fn wavy_underline_coverage(underline: Underline, position: vec2f) -> f32 {
    let normalized_position = (position - underline.bounds.origin) / underline.bounds.size.y - vec2f(0.0, 0.5);
    let angular_frequency = PI * UNDERLINE_WAVE_FREQUENCY * underline.thickness / underline.bounds.size.y;
    let amplitude = underline.thickness * UNDERLINE_WAVE_HEIGHT_RATIO / underline.bounds.size.y;
    let wave_height = sin(normalized_position.x * angular_frequency) * amplitude;
    let wave_slope = cos(normalized_position.x * angular_frequency) * amplitude * angular_frequency;
    let signed_distance = (normalized_position.y - wave_height) / sqrt(1.0 + wave_slope * wave_slope) * underline.bounds.size.y;
    let stroke_distance = abs(signed_distance) - underline.thickness * 0.5;
    return antialiased_coverage(stroke_distance);
}

struct UnderlineVarying {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) color: vec4f,
    @location(1) @interpolate(flat) underline_id: u32,
    @location(3) clip_distances: vec4f
}

@vertex fn vertex_underline(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> UnderlineVarying {
    let underline = UNDERLINES[u32(instance_id)];
    let vertex = rectangle_vertex(vertex_id, underline.bounds);
    return UnderlineVarying(vertex.clip_position, hsla_to_rgba(underline.color), instance_id, clip_distances(vertex.viewport_position, underline.content_mask.bounds));
}

@fragment fn fragment_underline(input: UnderlineVarying) -> @location(0) vec4f {
    if is_clipped(input.clip_distances) {
        return transparent();
    }
    let underline = UNDERLINES[u32(input.underline_id)];
    if !is_enabled(underline.wavy) {
        return blend_color(input.color, ContentMask_alpha(underline.content_mask, input.position.xy));
    }
    return blend_color(input.color, wavy_underline_coverage(underline, input.position.xy) * ContentMask_alpha(underline.content_mask, input.position.xy));
}
