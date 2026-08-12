use crate::capture;
use image::{imageops::FilterType, RgbaImage};
use ort::{
    environment::GlobalThreadPoolOptions,
    ep,
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const DETECTION_MODEL: &str = "PP-OCRv5_mobile_det_infer.onnx";
const RECOGNITION_MODEL: &str = "PP-OCRv5_mobile_rec_infer.onnx";
const DICTIONARY: &str = "ppocrv5_dict.txt";
const RUNTIME_LIBRARY: &str = "onnxruntime.dll";
const RUNTIME_DEPENDENCIES: [&str; 4] = [
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "msvcp140.dll",
    "msvcp140_1.dll",
];
// OCR is triggered intermittently, so retaining a large pool of worker threads is
// not worthwhile. Two threads preserve reasonable latency without the per-thread
// working memory of ORT's machine-sized default pool.
const OCR_INTRA_OP_THREADS: usize = 2;
const MAX_DETECTION_BOXES: usize = 256;
const MAX_RECOGNITION_WIDTH: u32 = 4096;

static OCR_ENVIRONMENT: OnceLock<Mutex<bool>> = OnceLock::new();

fn initialize_ocr_environment(model_dir: &Path) -> Result<(), String> {
    let runtime_path = resolve_runtime_path(model_dir);
    let initialized = OCR_ENVIRONMENT.get_or_init(|| Mutex::new(false));
    let mut initialized = initialized
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *initialized {
        return Ok(());
    }
    let result = (|| {
        let runtime_dir = runtime_path
            .parent()
            .ok_or_else(|| "The OCR runtime directory is invalid".to_owned())?;
        for dependency in RUNTIME_DEPENDENCIES {
            let dependency_path = runtime_dir.join(dependency);
            if !dependency_path.is_file() {
                return Err(format!(
                    "The OCR runtime dependency is missing: {}",
                    dependency_path.display()
                ));
            }
            ort::util::preload_dylib(&dependency_path).map_err(|error| {
                format!(
                    "Cannot load the OCR runtime dependency {}: {error}",
                    dependency_path.display()
                )
            })?;
        }
        // Load the pinned CPU runtime before constructing any ORT-backed
        // option object. Creating GlobalThreadPoolOptions first would make
        // `ort` resolve an arbitrary system onnxruntime.dll via its default
        // loader, defeating both version pinning and lazy loading.
        let environment = ort::init_from(&runtime_path).map_err(|error| {
            format!(
                "Cannot load the OCR runtime at {}: {error}",
                runtime_path.display()
            )
        })?;
        let thread_pool = GlobalThreadPoolOptions::default()
            .with_intra_threads(OCR_INTRA_OP_THREADS)
            .map_err(|error| format!("Cannot limit OCR worker threads: {error}"))?
            .with_inter_threads(1)
            .map_err(|error| format!("Cannot limit OCR inter-op threads: {error}"))?
            .with_spin_control(false)
            .map_err(|error| format!("Cannot disable OCR worker spinning: {error}"))?;
        if environment
            .with_name("hd2-ocr")
            .with_telemetry(false)
            .with_global_thread_pool(thread_pool)
            .commit()
        {
            Ok(())
        } else {
            Err("ONNX Runtime was initialized before the OCR thread pool was configured".to_owned())
        }
    })();
    if result.is_ok() {
        *initialized = true;
    }
    result
}

fn resolve_runtime_path(model_dir: &Path) -> PathBuf {
    let resource_root = model_dir.parent().and_then(Path::parent);
    let candidates = [
        Some(model_dir.join(RUNTIME_LIBRARY)),
        resource_root.map(|root| root.join("runtime").join(RUNTIME_LIBRARY)),
        resource_root.map(|root| root.join(RUNTIME_LIBRARY)),
    ];
    candidates
        .iter()
        .flatten()
        .find(|candidate| candidate.is_file())
        .cloned()
        .or_else(|| candidates.into_iter().flatten().nth(1))
        .unwrap_or_else(|| model_dir.join(RUNTIME_LIBRARY))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub detection_model_loaded: bool,
    pub recognition_model_loaded: bool,
    pub dictionary_entries: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrText {
    pub text: String,
    pub confidence: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct OcrRegion {
    // Global desktop coordinates in physical pixels. Keeping this contract aligned
    // with screenshots::DisplayInfo avoids DPI conversion errors at 125%/150%.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrDisplay {
    pub id: u32,
    pub index: usize,
    pub bounds: OcrBounds,
    pub scale_factor: f32,
    pub is_primary: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OcrBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
struct RecognizedBox {
    text: String,
    confidence: f32,
    rect: Rect,
}

pub struct OcrEngine {
    detection: Session,
    recognition: Session,
    dictionary: OcrDictionary,
}

struct OcrDictionary {
    text: String,
    entries: Vec<(u32, u32)>,
}

impl OcrDictionary {
    fn from_contents(contents: &str) -> Result<Self, String> {
        let mut text = String::with_capacity(contents.len());
        let mut entries = Vec::with_capacity(contents.lines().count().saturating_add(1));
        entries.push((0, 0));
        for line in contents.lines().filter(|line| !line.is_empty()) {
            let start =
                u32::try_from(text.len()).map_err(|_| "OCR dictionary is too large".to_owned())?;
            text.push_str(line);
            let end =
                u32::try_from(text.len()).map_err(|_| "OCR dictionary is too large".to_owned())?;
            entries.push((start, end));
        }
        Ok(Self { text, entries })
    }

    fn get(&self, index: usize) -> Option<&str> {
        let (start, end) = *self.entries.get(index)?;
        self.text.get(start as usize..end as usize)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
pub fn verify_models() -> Result<ModelStatus, String> {
    verify_models_in(development_model_directory())
}

#[cfg(test)]
pub fn verify_models_in(model_dir: PathBuf) -> Result<ModelStatus, String> {
    let engine = OcrEngine::load_from_directory(model_dir)?;
    Ok(engine.model_status())
}

pub fn model_files_exist(model_dir: &Path) -> bool {
    [DETECTION_MODEL, RECOGNITION_MODEL, DICTIONARY]
        .iter()
        .all(|filename| model_dir.join(filename).is_file())
}

impl OcrEngine {
    pub fn model_status(&self) -> ModelStatus {
        ModelStatus {
            detection_model_loaded: true,
            recognition_model_loaded: true,
            dictionary_entries: self.dictionary.len(),
        }
    }

    #[cfg(test)]
    pub fn load_development() -> Result<Self, String> {
        Self::load_from_directory(development_model_directory())
    }

    pub fn load_from_directory(model_dir: PathBuf) -> Result<Self, String> {
        initialize_ocr_environment(&model_dir)?;
        let detection_path = model_dir.join(DETECTION_MODEL);
        let recognition_path = model_dir.join(RECOGNITION_MODEL);
        let dictionary_path = model_dir.join(DICTIONARY);

        // Match the Electron version's ONNX Runtime configuration: disable memory
        // pattern (dynamic input sizes) and use sequential execution mode. Unlike
        // the default CPU provider, do not use ORT's arena allocator: it keeps the
        // largest first-inference workspace reserved for the lifetime of a session.
        // This app's OCR is intermittent, so returning that workspace after each
        // recognition is a better memory/latency trade-off.
        let detection = Session::builder()
            .map_err(|error| format!("Cannot create detection session: {error}"))?
            .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])
            .map_err(|error| format!("Cannot configure detection CPU provider: {error}"))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|error| format!("Cannot set detection optimization: {error}"))?
            .with_memory_pattern(false)
            .map_err(|error| format!("Cannot disable detection mem pattern: {error}"))?
            .with_parallel_execution(false)
            .map_err(|error| format!("Cannot set detection sequential mode: {error}"))?
            .commit_from_file(&detection_path)
            .map_err(|error| format!("Cannot load {DETECTION_MODEL}: {error}"))?;
        let recognition = Session::builder()
            .map_err(|error| format!("Cannot create recognition session: {error}"))?
            .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])
            .map_err(|error| format!("Cannot configure recognition CPU provider: {error}"))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|error| format!("Cannot set recognition optimization: {error}"))?
            .with_memory_pattern(false)
            .map_err(|error| format!("Cannot disable recognition mem pattern: {error}"))?
            .with_parallel_execution(false)
            .map_err(|error| format!("Cannot set recognition sequential mode: {error}"))?
            .commit_from_file(&recognition_path)
            .map_err(|error| format!("Cannot load {RECOGNITION_MODEL}: {error}"))?;
        let dictionary_contents = fs::read_to_string(dictionary_path)
            .map_err(|error| format!("Cannot read OCR dictionary: {error}"))?;
        let dictionary = OcrDictionary::from_contents(&dictionary_contents)?;

        Ok(Self {
            detection,
            recognition,
            dictionary,
        })
    }

    pub fn detect(&mut self, image: &RgbaImage) -> Result<Vec<Rect>, String> {
        let (resized, source_width, source_height) = resize_for_detection(image, 1536)?;
        let (width, height) = resized.dimensions();
        let tensor = normalized_tensor(
            &resized,
            [0.485 * 255.0, 0.456 * 255.0, 0.406 * 255.0],
            [
                1.0 / 0.229 / 255.0,
                1.0 / 0.224 / 255.0,
                1.0 / 0.255 / 255.0,
            ],
        );
        let input = Tensor::from_array((
            [1_usize, 3, height as usize, width as usize],
            tensor.into_boxed_slice(),
        ))
        .map_err(|error| format!("Cannot build detection tensor: {error}"))?;
        drop(resized);
        let output = self
            .detection
            .run(ort::inputs![input])
            .map_err(|error| format!("Detection inference failed: {error}"))?;
        let (_, output_value) = output
            .iter()
            .next()
            .ok_or_else(|| "Detection model returned no output".to_owned())?;
        let (shape, scores) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("Cannot read detection result: {error}"))?;
        let dims = &**shape;
        if dims.len() != 4 {
            return Err("Unexpected detection output shape".to_owned());
        }
        let output_width = positive_dimension(dims, 3, "detection width")?;
        let output_height = positive_dimension(dims, 2, "detection height")?;
        let score_count = output_width
            .checked_mul(output_height)
            .ok_or_else(|| "Detection output dimensions overflow".to_owned())?;
        let score_slice = scores
            .get(..score_count)
            .ok_or_else(|| "Detection output buffer is shorter than its shape".to_owned())?;
        let mut binary = Vec::new();
        binary
            .try_reserve_exact(score_count)
            .map_err(|_| "Detection result is too large for available memory".to_owned())?;
        for score in score_slice {
            if !score.is_finite() {
                return Err("Detection model returned a non-finite score".to_owned());
            }
            binary.push(u8::from((score * 255.0).round() > 63.75));
        }
        drop(output);
        let output_width_u32 = u32::try_from(output_width)
            .map_err(|_| "Detection output width is unsupported".to_owned())?;
        let output_height_u32 = u32::try_from(output_height)
            .map_err(|_| "Detection output height is unsupported".to_owned())?;
        let contours = contours(
            dilate(binary, output_width_u32, output_height_u32),
            output_width_u32,
            output_height_u32,
            6,
        );
        if contours.len() > MAX_DETECTION_BOXES {
            return Err(format!(
                "Detection produced too many text regions ({}/{MAX_DETECTION_BOXES})",
                contours.len()
            ));
        }
        let scale_x = width as f32 / source_width as f32;
        let scale_y = height as f32 / source_height as f32;
        let boxes = contours
            .into_iter()
            .map(|rect| {
                let scaled = Rect {
                    x: rect.x.saturating_mul(width) / output_width_u32,
                    y: rect.y.saturating_mul(height) / output_height_u32,
                    width: (rect.width.saturating_mul(width) / output_width_u32).max(1),
                    height: (rect.height.saturating_mul(height) / output_height_u32).max(1),
                };
                padded_and_unscaled(
                    scaled,
                    width,
                    height,
                    source_width,
                    source_height,
                    scale_x,
                    scale_y,
                )
            })
            .collect::<Vec<_>>();
        // The Electron implementation deliberately treats a missed detector pass
        // as one full-frame text box. Keep that compatibility fallback intact.
        if boxes.is_empty() {
            Ok(vec![Rect {
                x: 0,
                y: 0,
                width: source_width,
                height: source_height,
            }])
        } else {
            Ok(boxes)
        }
    }

    pub fn recognize(&mut self, image: &RgbaImage) -> Result<OcrText, String> {
        let boxes = self.detect(image)?;
        let mut recognized = Vec::with_capacity(boxes.len());
        for rect in boxes
            .into_iter()
            .filter(|rect| rect.width > 0 && rect.height > 0)
        {
            let target_width = ((rect.width as f32 / rect.height as f32) * 48.0)
                .round()
                .clamp(1.0, MAX_RECOGNITION_WIDTH as f32) as u32;
            let resized = resize_recognition_region(image, rect, target_width);
            let tensor = normalized_tensor(&resized, [127.5; 3], [1.0 / 127.5; 3]);
            let input = Tensor::from_array((
                [1_usize, 3, 48, target_width as usize],
                tensor.into_boxed_slice(),
            ))
            .map_err(|error| format!("Cannot build recognition tensor: {error}"))?;
            drop(resized);
            let output = self
                .recognition
                .run(ort::inputs![input])
                .map_err(|error| format!("Recognition inference failed: {error}"))?;
            let (_, output_value) = output
                .iter()
                .next()
                .ok_or_else(|| "Recognition model returned no output".to_owned())?;
            let (shape, logits) = output_value
                .try_extract_tensor::<f32>()
                .map_err(|error| format!("Cannot read recognition result: {error}"))?;
            let dims = &**shape;
            if dims.len() != 3 {
                return Err("Unexpected recognition output shape".to_owned());
            }
            let sequence_length = positive_dimension(dims, 1, "recognition sequence")?;
            let classes = positive_dimension(dims, 2, "recognition classes")?;
            let (text, confidence) =
                ctc_decode(logits, sequence_length, classes, &self.dictionary)?;
            recognized.push(RecognizedBox {
                text,
                confidence,
                rect,
            });
        }
        // A pair-dependent "same line" comparator is non-transitive when
        // boxes overlap several rows and can make Rust's sort panic. Keep the
        // ordering total and let `join_reading_order` perform line grouping.
        sort_recognized_boxes(&mut recognized);
        Ok(join_reading_order(recognized))
    }

    pub fn recognize_region(&mut self, mut image: RgbaImage) -> Result<OcrText, String> {
        if image.width() == 0 || image.height() == 0 {
            return Err("OCR image is empty".to_owned());
        }
        apply_legacy_contrast(&mut image, 0.18);
        let max_side = image.width().max(image.height());
        let scale = (1200 / max_side).clamp(1, 4);
        if scale > 1 {
            let width = image
                .width()
                .checked_mul(scale)
                .ok_or_else(|| "OCR image width overflows while scaling".to_owned())?;
            let height = image
                .height()
                .checked_mul(scale)
                .ok_or_else(|| "OCR image height overflows while scaling".to_owned())?;
            image = image::imageops::resize(&image, width, height, FilterType::Triangle);
        }
        self.recognize(&image)
    }
}

fn apply_legacy_contrast(image: &mut RgbaImage, amount: f32) {
    // Jimp's contrast value is normalized to [-1, 1]. `image` uses a
    // different scale, so passing 0.18 directly barely changed the pixels.
    let amount = amount.clamp(-1.0, 1.0);
    let factor = (amount + 1.0) / (1.0 - amount);
    for pixel in image.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            *channel = (factor.mul_add(f32::from(*channel) - 127.0, 127.0))
                .floor()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

fn resize_recognition_region(image: &RgbaImage, rect: Rect, target_width: u32) -> RgbaImage {
    let crop = image::imageops::crop_imm(image, rect.x, rect.y, rect.width, rect.height);
    // Resize the SubImage itself. `SubImage::inner()` is the complete parent
    // image, not the selected rectangle; using it silently feeds a squashed
    // full-screen frame to every recognition inference.
    image::imageops::resize(&*crop, target_width, 48, FilterType::Triangle)
}

fn sort_recognized_boxes(recognized: &mut [RecognizedBox]) {
    recognized.sort_by_key(|result| {
        (
            result.rect.y,
            result.rect.x,
            result.rect.height,
            result.rect.width,
        )
    });
}

pub fn capture_region(region: OcrRegion) -> Result<RgbaImage, String> {
    if !region.x.is_finite()
        || !region.y.is_finite()
        || !region.width.is_finite()
        || !region.height.is_finite()
        || region.width <= 0.0
        || region.height <= 0.0
    {
        return Err("Invalid OCR region".to_owned());
    }
    let x = rounded_coordinate(region.x, "x")?;
    let y = rounded_coordinate(region.y, "y")?;
    let width = rounded_dimension(region.width, "width")?;
    let height = rounded_dimension(region.height, "height")?;
    capture::capture_rgba(x, y, width, height)
}

pub fn available_displays() -> Result<Vec<OcrDisplay>, String> {
    let displays = capture::displays()?
        .into_iter()
        .enumerate()
        .map(|(index, display)| OcrDisplay {
            id: display.id,
            index: index + 1,
            bounds: OcrBounds {
                x: display.x,
                y: display.y,
                width: display.width,
                height: display.height,
            },
            scale_factor: display.scale_factor,
            is_primary: display.is_primary,
        })
        .collect::<Vec<_>>();
    Ok(displays)
}

fn rounded_coordinate(value: f64, name: &str) -> Result<i32, String> {
    let rounded = value.round();
    if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        return Err(format!("OCR region {name} coordinate is unsupported"));
    }
    Ok(rounded as i32)
}

fn rounded_dimension(value: f64, name: &str) -> Result<u32, String> {
    let rounded = value.round();
    if rounded < 1.0 || rounded > u32::MAX as f64 {
        return Err(format!("OCR region {name} is unsupported"));
    }
    Ok(rounded as u32)
}

fn positive_dimension(dims: &[i64], index: usize, name: &str) -> Result<usize, String> {
    dims.get(index)
        .copied()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("Model {name} dimension is invalid"))
}

fn ctc_decode(
    logits: &[f32],
    sequence_length: usize,
    classes: usize,
    dictionary: &OcrDictionary,
) -> Result<(String, f32), String> {
    if sequence_length == 0 || classes == 0 {
        return Err("Recognition output dimensions must be positive".to_owned());
    }
    let required = sequence_length
        .checked_mul(classes)
        .ok_or_else(|| "Recognition output dimensions overflow".to_owned())?;
    let logits = logits
        .get(..required)
        .ok_or_else(|| "Recognition output buffer is shorter than its shape".to_owned())?;
    let mut text = String::new();
    let mut score_sum = 0.0_f32;
    let mut score_count = 0_usize;
    for row in logits.chunks_exact(classes).take(sequence_length) {
        let mut max_score = 0.0_f32;
        let mut max_index = 0_usize;
        for (index, score) in row.iter().enumerate() {
            if !score.is_finite() {
                return Err("Recognition model returned a non-finite score".to_owned());
            }
            if *score > max_score {
                max_score = *score;
                max_index = index;
            }
        }
        if max_index != 0 {
            text.push_str(dictionary.get(max_index).unwrap_or(""));
            score_sum += max_score;
            score_count += 1;
        }
    }
    let confidence = if score_count == 0 {
        0.0
    } else {
        score_sum / score_count as f32
    };
    Ok((text, confidence))
}

fn join_reading_order(recognized: Vec<RecognizedBox>) -> OcrText {
    if recognized.is_empty() {
        return OcrText {
            text: String::new(),
            confidence: 0.0,
        };
    }
    let confidence = recognized
        .iter()
        .map(|result| result.confidence)
        .sum::<f32>()
        / recognized.len() as f32;
    let text_capacity = recognized
        .iter()
        .map(|result| result.text.len().saturating_add(1))
        .sum();
    let mut text = String::with_capacity(text_capacity);
    text.push_str(&recognized[0].text);
    let mut previous = &recognized[0];
    let mut line_height_sum = recognized[0].rect.height as f32;
    let mut line_count = 1_usize;
    for current in recognized.iter().skip(1) {
        let vertical_gap = (current.rect.y as i64 - previous.rect.y as i64).unsigned_abs() as f32;
        let average_height = line_height_sum / line_count as f32;
        if vertical_gap <= average_height * 0.5 {
            text.push(' ');
            text.push_str(&current.text);
            line_height_sum += current.rect.height as f32;
            line_count += 1;
        } else {
            text.push('\n');
            text.push_str(&current.text);
            line_height_sum = current.rect.height as f32;
            line_count = 1;
        }
        previous = current;
    }
    OcrText { text, confidence }
}

fn resize_for_detection(image: &RgbaImage, max_side: u32) -> Result<(RgbaImage, u32, u32), String> {
    let (source_width, source_height) = image.dimensions();
    if source_width == 0 || source_height == 0 || max_side == 0 {
        return Err("Detection image dimensions are invalid".to_owned());
    }
    let ratio = max_side as f32 / source_width.max(source_height) as f32;
    // The original Electron pipeline rounds each axis down independently.
    // PP-OCR's internal Resize nodes require both spatial dimensions to be
    // multiples of 32; coupling the height rounding to the width caused valid
    // wide/tall selections to fail whenever only one axis needed rounding.
    let width = (((source_width as f32 * ratio).floor().max(1.0) as u32) / 32).max(1) * 32;
    let height = (((source_height as f32 * ratio).floor().max(1.0) as u32) / 32).max(1) * 32;
    Ok((
        image::imageops::resize(image, width, height, FilterType::Triangle),
        source_width,
        source_height,
    ))
}

fn normalized_tensor(image: &RgbaImage, mean: [f32; 3], norm: [f32; 3]) -> Vec<f32> {
    let (width, height) = image.dimensions();
    let plane = (width * height) as usize;
    let mut tensor = vec![0.0; plane * 3];
    let (red, remainder) = tensor.split_at_mut(plane);
    let (green, blue) = remainder.split_at_mut(plane);
    let bias = [mean[0] * norm[0], mean[1] * norm[1], mean[2] * norm[2]];
    for (index, pixel) in image.pixels().enumerate() {
        red[index] = pixel[0] as f32 * norm[0] - bias[0];
        green[index] = pixel[1] as f32 * norm[1] - bias[1];
        blue[index] = pixel[2] as f32 * norm[2] - bias[2];
    }
    tensor
}

fn dilate(input: Vec<u8>, width: u32, height: u32) -> Vec<u8> {
    let Some(expected_len) = (width as usize).checked_mul(height as usize) else {
        return Vec::new();
    };
    if width == 0 || height == 0 || input.len() != expected_len {
        return Vec::new();
    }
    let width_usize = width as usize;
    let height_usize = height as usize;
    let mut horizontal = vec![0; input.len()];
    for y in 0..height_usize {
        let row_start = y * width_usize;
        for x in 0..width_usize {
            let start = x.saturating_sub(1);
            let end = (x + 1).min(width_usize - 1);
            horizontal[row_start + x] = u8::from(
                input[row_start + start..=row_start + end]
                    .iter()
                    .any(|value| *value != 0),
            );
        }
    }
    drop(input);

    let mut output = vec![0; horizontal.len()];
    for y in 0..height {
        for x in 0..width {
            let start_y = y.saturating_sub(1) as usize;
            let end_y = (y + 1).min(height - 1) as usize;
            let x = x as usize;
            let active = (start_y..=end_y).any(|row| horizontal[row * width_usize + x] != 0);
            output[y as usize * width as usize + x] = u8::from(active);
        }
    }
    output
}

fn contours(mut input: Vec<u8>, width: u32, height: u32, min_area: u32) -> Vec<Rect> {
    let Some(expected_len) = (width as usize).checked_mul(height as usize) else {
        return Vec::new();
    };
    if width == 0 || height == 0 || input.len() != expected_len {
        return Vec::new();
    }
    let mut boxes = Vec::new();
    let mut stack = Vec::with_capacity(128);
    for y in 0..height {
        for x in 0..width {
            let start = y as usize * width as usize + x as usize;
            if input[start] == 0 {
                continue;
            }
            input[start] = 0;
            stack.push((x, y));
            let (mut min_x, mut min_y, mut max_x, mut max_y, mut area) = (x, y, x, y, 0_u32);
            while let Some((current_x, current_y)) = stack.pop() {
                area += 1;
                min_x = min_x.min(current_x);
                min_y = min_y.min(current_y);
                max_x = max_x.max(current_x);
                max_y = max_y.max(current_y);
                for dy in -1_i32..=1 {
                    for dx in -1_i32..=1 {
                        let nx = current_x as i32 + dx;
                        let ny = current_y as i32 + dy;
                        if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                            continue;
                        }
                        let index = ny as usize * width as usize + nx as usize;
                        if input[index] != 0 {
                            input[index] = 0;
                            stack.push((nx as u32, ny as u32));
                        }
                    }
                }
            }
            if area >= min_area {
                boxes.push(Rect {
                    x: min_x,
                    y: min_y,
                    width: max_x - min_x + 1,
                    height: max_y - min_y + 1,
                });
            }
        }
    }
    boxes
}

fn padded_and_unscaled(
    rect: Rect,
    max_width: u32,
    max_height: u32,
    source_width: u32,
    source_height: u32,
    scale_x: f32,
    scale_y: f32,
) -> Rect {
    let vertical = (rect.height as f32 * 0.45).round() as u32;
    let horizontal = (rect.height as f32 * 0.6).round() as u32;
    let x = rect.x.saturating_sub(horizontal);
    let y = rect.y.saturating_sub(vertical);
    let right = (rect.x + rect.width + horizontal).min(max_width);
    let bottom = (rect.y + rect.height + vertical).min(max_height);
    let unscaled_x = (x as f32 / scale_x).round().max(0.0) as u32;
    let unscaled_y = (y as f32 / scale_y).round().max(0.0) as u32;
    Rect {
        x: unscaled_x,
        y: unscaled_y,
        width: ((right - x) as f32 / scale_x)
            .round()
            .min(source_width.saturating_sub(unscaled_x) as f32) as u32,
        height: ((bottom - y) as f32 / scale_y)
            .round()
            .min(source_height.saturating_sub(unscaled_y) as f32) as u32,
    }
}

#[cfg(test)]
fn development_model_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/models/ocr")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_dilate(input: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut output = vec![0; input.len()];
        for y in 0..height {
            for x in 0..width {
                for dy in -1_i32..=1 {
                    for dx in -1_i32..=1 {
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        if nx >= 0
                            && ny >= 0
                            && nx < width as i32
                            && ny < height as i32
                            && input[ny as usize * width as usize + nx as usize] != 0
                        {
                            output[y as usize * width as usize + x as usize] = 1;
                        }
                    }
                }
            }
        }
        output
    }

    fn reference_contours(input: &[u8], width: u32, height: u32, min_area: u32) -> Vec<Rect> {
        let mut visited = vec![false; input.len()];
        let mut boxes = Vec::new();
        let mut stack = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let start = y as usize * width as usize + x as usize;
                if input[start] == 0 || visited[start] {
                    continue;
                }
                visited[start] = true;
                stack.push((x, y));
                let (mut min_x, mut min_y, mut max_x, mut max_y, mut area) = (x, y, x, y, 0_u32);
                while let Some((current_x, current_y)) = stack.pop() {
                    area += 1;
                    min_x = min_x.min(current_x);
                    min_y = min_y.min(current_y);
                    max_x = max_x.max(current_x);
                    max_y = max_y.max(current_y);
                    for dy in -1_i32..=1 {
                        for dx in -1_i32..=1 {
                            let nx = current_x as i32 + dx;
                            let ny = current_y as i32 + dy;
                            if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                                continue;
                            }
                            let index = ny as usize * width as usize + nx as usize;
                            if input[index] != 0 && !visited[index] {
                                visited[index] = true;
                                stack.push((nx as u32, ny as u32));
                            }
                        }
                    }
                }
                if area >= min_area {
                    boxes.push(Rect {
                        x: min_x,
                        y: min_y,
                        width: max_x - min_x + 1,
                        height: max_y - min_y + 1,
                    });
                }
            }
        }
        boxes
    }

    #[test]
    #[ignore = "diagnostic display snapshot"]
    fn reports_display_contract() {
        for display in available_displays().expect("displays should enumerate") {
            println!("{display:?}");
        }
    }

    #[test]
    #[ignore = "manual OCR hot-path benchmark"]
    fn benchmark_ocr_hot_paths() {
        use std::time::Instant;

        let image = RgbaImage::from_pixel(1536, 864, image::Rgba([37, 91, 173, 255]));
        let started = Instant::now();
        for _ in 0..5 {
            std::hint::black_box(normalized_tensor(
                &image,
                [0.485 * 255.0, 0.456 * 255.0, 0.406 * 255.0],
                [
                    1.0 / 0.229 / 255.0,
                    1.0 / 0.224 / 255.0,
                    1.0 / 0.255 / 255.0,
                ],
            ));
        }
        println!("normalized_5={:?}", started.elapsed());

        let mut binary = vec![0_u8; 1536 * 864];
        for y in (4..860).step_by(11) {
            for x in (4..1532).step_by(17) {
                binary[y * 1536 + x] = 1;
            }
        }
        let started = Instant::now();
        let mut expanded = Vec::new();
        for _ in 0..5 {
            expanded = dilate(binary.clone(), 1536, 864);
            std::hint::black_box(&expanded);
        }
        println!("dilate_5={:?}", started.elapsed());

        let started = Instant::now();
        let boxes = contours(expanded, 1536, 864, 6);
        println!("contours_1={:?} boxes={}", started.elapsed(), boxes.len());
    }

    #[test]
    #[ignore = "manual desktop capture benchmark"]
    fn benchmark_desktop_capture() {
        use std::time::Instant;

        let display = available_displays()
            .expect("displays should enumerate")
            .into_iter()
            .next()
            .expect("a display should exist");
        let region = OcrRegion {
            x: display.bounds.x as f64,
            y: display.bounds.y as f64,
            width: 640.0_f64.min(display.bounds.width as f64),
            height: 360.0_f64.min(display.bounds.height as f64),
        };
        let started = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(capture_region(region).expect("capture should succeed"));
        }
        println!("capture_100={:?}", started.elapsed());
    }

    #[test]
    fn bundled_ocr_assets_exist() {
        let model_dir = development_model_directory();
        assert!(model_dir.join(DETECTION_MODEL).is_file());
        assert!(model_dir.join(RECOGNITION_MODEL).is_file());
        assert!(model_dir.join(DICTIONARY).is_file());
        let runtime = resolve_runtime_path(&model_dir);
        assert!(runtime.is_file());
        for dependency in RUNTIME_DEPENDENCIES {
            assert!(runtime.with_file_name(dependency).is_file());
        }
        assert_eq!(
            fs::read_to_string(runtime.with_file_name("VERSION_NUMBER"))
                .expect("runtime version marker should be readable")
                .trim(),
            "1.24.4"
        );
    }

    #[test]
    fn models_open_with_the_native_onnx_runtime() {
        let status = verify_models().expect("bundled OCR models should load");
        assert!(status.detection_model_loaded);
        assert!(status.recognition_model_loaded);
        assert!(status.dictionary_entries > 18_000);
    }

    #[test]
    #[ignore = "set HD2_INSTALL_SMOKE_DIR to an extracted or installed bundle"]
    fn installed_bundle_opens_its_own_runtime_and_models() {
        let install_root = std::env::var_os("HD2_INSTALL_SMOKE_DIR")
            .map(PathBuf::from)
            .expect("HD2_INSTALL_SMOKE_DIR must point at the bundle root");
        let model_dir = install_root.join("models/ocr");
        let runtime = resolve_runtime_path(&model_dir);
        assert_eq!(runtime, install_root.join("runtime/onnxruntime.dll"));
        let status = OcrEngine::load_from_directory(model_dir)
            .expect("the installed runtime and models should load")
            .model_status();
        assert!(status.detection_model_loaded);
        assert!(status.recognition_model_loaded);
        assert!(status.dictionary_entries > 18_000);
    }

    #[test]
    fn detector_accepts_a_rgba_frame() {
        let mut engine = OcrEngine::load_development().expect("models should load");
        let frame = RgbaImage::new(64, 64);
        let boxes = engine
            .detect(&frame)
            .expect("detector should execute on a frame");
        assert!(
            !boxes.is_empty(),
            "detector fallback keeps Electron's full-frame behavior"
        );
    }

    #[test]
    fn ctc_decoder_keeps_legacy_non_deduplicating_behavior() {
        let dictionary =
            OcrDictionary::from_contents("A\nB\n").expect("test dictionary should be valid");
        let (text, confidence) = ctc_decode(&[0.0, 0.8, 0.1, 0.0, 0.7, 0.2], 2, 3, &dictionary)
            .expect("valid logits should decode");
        assert_eq!(text, "AA");
        assert!((confidence - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn ctc_decoder_rejects_malformed_model_outputs() {
        let dictionary =
            OcrDictionary::from_contents("A\nB\n").expect("test dictionary should be valid");
        assert!(ctc_decode(&[], 1, 0, &dictionary).is_err());
        assert!(ctc_decode(&[0.0, 0.8], 2, 2, &dictionary).is_err());
        assert!(ctc_decode(&[0.0, f32::NAN, 0.2], 1, 3, &dictionary).is_err());
    }

    #[test]
    fn optimized_morphology_matches_the_reference_implementation() {
        let (width, height) = (47_u32, 31_u32);
        let mut seed = 0xC0FF_EE11_u32;
        let mut input = vec![0_u8; (width * height) as usize];
        for value in &mut input {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *value = u8::from(seed.is_multiple_of(13));
        }
        let expected_dilation = reference_dilate(&input, width, height);
        let actual_dilation = dilate(input, width, height);
        assert_eq!(actual_dilation, expected_dilation);
        assert_eq!(
            contours(actual_dilation.clone(), width, height, 3),
            reference_contours(&actual_dilation, width, height, 3)
        );
    }

    #[test]
    fn morphology_rejects_invalid_layouts() {
        assert!(dilate(vec![1], 0, 1).is_empty());
        assert!(dilate(vec![1], 2, 2).is_empty());
        assert!(contours(vec![1], 0, 1, 1).is_empty());
        assert!(contours(vec![1], 2, 2, 1).is_empty());
    }

    #[test]
    fn detector_resize_rounds_both_axes_like_the_legacy_pipeline() {
        let image = RgbaImage::new(320, 96);
        let (resized, source_width, source_height) =
            resize_for_detection(&image, 1536).expect("valid image should resize");
        assert_eq!((source_width, source_height), (320, 96));
        assert_eq!(resized.dimensions(), (1536, 448));
        assert!(resized.width().is_multiple_of(32));
        assert!(resized.height().is_multiple_of(32));
    }

    #[test]
    fn recognition_resize_uses_the_selected_crop_not_the_parent_image() {
        let image = RgbaImage::from_fn(4, 2, |x, _| {
            if x < 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let resized = resize_recognition_region(
            &image,
            Rect {
                x: 2,
                y: 0,
                width: 2,
                height: 2,
            },
            2,
        );
        assert_eq!(resized.dimensions(), (2, 48));
        assert!(resized.pixels().all(|pixel| pixel.0 == [0, 0, 255, 255]));
    }

    #[test]
    fn preprocessing_keeps_the_legacy_jimp_contrast_scale() {
        let mut image = RgbaImage::from_pixel(1, 1, image::Rgba([100, 127, 200, 17]));
        apply_legacy_contrast(&mut image, 0.18);
        assert_eq!(image.get_pixel(0, 0).0, [88, 127, 232, 17]);
    }

    #[test]
    fn reading_order_sort_is_total_for_overlapping_rows() {
        let mut boxes = vec![
            RecognizedBox {
                text: "right".to_owned(),
                confidence: 1.0,
                rect: Rect {
                    x: 20,
                    y: 10,
                    width: 10,
                    height: 30,
                },
            },
            RecognizedBox {
                text: "top".to_owned(),
                confidence: 1.0,
                rect: Rect {
                    x: 50,
                    y: 0,
                    width: 10,
                    height: 80,
                },
            },
            RecognizedBox {
                text: "left".to_owned(),
                confidence: 1.0,
                rect: Rect {
                    x: 5,
                    y: 10,
                    width: 10,
                    height: 5,
                },
            },
        ];
        sort_recognized_boxes(&mut boxes);
        assert_eq!(
            boxes
                .iter()
                .map(|box_result| box_result.text.as_str())
                .collect::<Vec<_>>(),
            ["top", "left", "right"]
        );
    }

    #[test]
    fn region_capturer_reads_the_active_desktop() {
        let display = available_displays().expect("a desktop display should exist")[0].clone();
        let image = capture_region(OcrRegion {
            x: display.bounds.x as f64,
            y: display.bounds.y as f64,
            width: 16.0,
            height: 16.0,
        })
        .expect("desktop region should be capturable");
        assert_eq!(image.width(), 16);
    }

    #[test]
    fn native_pipeline_runs_from_capture_to_text() {
        let display = available_displays().expect("a desktop display should exist")[0].clone();
        let frame = capture_region(OcrRegion {
            x: display.bounds.x as f64,
            y: display.bounds.y as f64,
            width: 160.0,
            height: 80.0,
        })
        .expect("desktop region should be capturable");
        let mut engine = OcrEngine::load_development().expect("models should load");
        let result = engine
            .recognize_region(frame)
            .expect("native OCR pipeline should complete");
        assert!(result.confidence.is_finite());
    }

    #[test]
    fn synthetic_detection_fingerprint_and_recognition_stay_safe() {
        let mut frame = RgbaImage::from_pixel(320, 160, image::Rgba([245, 245, 245, 255]));
        for (left, top, width, height) in [
            (24, 18, 12, 60),
            (60, 18, 12, 60),
            (24, 42, 48, 12),
            (112, 18, 12, 60),
            (112, 18, 42, 12),
            (112, 66, 42, 12),
            (172, 18, 52, 12),
            (212, 18, 12, 30),
            (172, 42, 52, 12),
            (172, 42, 12, 36),
            (172, 66, 52, 12),
        ] {
            for y in top..top + height {
                for x in left..left + width {
                    frame.put_pixel(x, y, image::Rgba([12, 12, 12, 255]));
                }
            }
        }
        let mut engine = OcrEngine::load_development().expect("models should load");
        let boxes = engine.detect(&frame).expect("detector should run");
        let result = engine
            .recognize_region(frame)
            .expect("recognition should run");
        assert_eq!(
            boxes,
            vec![
                Rect {
                    x: 34,
                    y: 56,
                    width: 17,
                    height: 16,
                },
                Rect {
                    x: 40,
                    y: 56,
                    width: 23,
                    height: 19,
                },
                Rect {
                    x: 58,
                    y: 59,
                    width: 15,
                    height: 17,
                },
                Rect {
                    x: 26,
                    y: 64,
                    width: 10,
                    height: 10,
                },
                Rect {
                    x: 42,
                    y: 69,
                    width: 2,
                    height: 1,
                },
                Rect {
                    x: 51,
                    y: 70,
                    width: 2,
                    height: 1,
                },
            ]
        );
        assert!(result.confidence.is_finite());
        assert!((0.0..=1.0).contains(&result.confidence));
    }
}
