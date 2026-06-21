use image::{imageops::FilterType, RgbaImage};
use ort::{
    ep,
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

const DETECTION_MODEL: &str = "PP-OCRv5_mobile_det_infer.onnx";
const RECOGNITION_MODEL: &str = "PP-OCRv5_mobile_rec_infer.onnx";
const DICTIONARY: &str = "ppocrv5_dict.txt";
// OCR is triggered intermittently, so retaining a large pool of worker threads is
// not worthwhile. Two threads preserve reasonable latency without the per-thread
// working memory of ORT's machine-sized default pool.
const OCR_INTRA_OP_THREADS: usize = 2;

#[derive(Serialize)]
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
    dictionary: Vec<String>,
}

#[cfg(test)]
pub fn verify_models() -> Result<ModelStatus, String> {
    verify_models_in(development_model_directory())
}

pub fn verify_models_in(model_dir: PathBuf) -> Result<ModelStatus, String> {
    let engine = OcrEngine::load_from_directory(model_dir)?;
    Ok(ModelStatus {
        detection_model_loaded: true,
        recognition_model_loaded: true,
        dictionary_entries: engine.dictionary.len(),
    })
}

impl OcrEngine {
    #[cfg(test)]
    pub fn load_development() -> Result<Self, String> {
        Self::load_from_directory(development_model_directory())
    }

    pub fn load_from_directory(model_dir: PathBuf) -> Result<Self, String> {
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
            .with_intra_threads(OCR_INTRA_OP_THREADS)
            .map_err(|error| format!("Cannot set detection thread limit: {error}"))?
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
            .with_intra_threads(OCR_INTRA_OP_THREADS)
            .map_err(|error| format!("Cannot set recognition thread limit: {error}"))?
            .commit_from_file(&recognition_path)
            .map_err(|error| format!("Cannot load {RECOGNITION_MODEL}: {error}"))?;
        let mut dictionary = vec![String::new()];
        dictionary.extend(
            fs::read_to_string(dictionary_path)
                .map_err(|error| format!("Cannot read OCR dictionary: {error}"))?
                .lines()
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned),
        );

        Ok(Self {
            detection,
            recognition,
            dictionary,
        })
    }

    pub fn detect(&mut self, image: &RgbaImage) -> Result<Vec<Rect>, String> {
        let (resized, source_width, source_height) = resize_for_detection(image, 1536);
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
        let output = self
            .detection
            .run(ort::inputs![input])
            .map_err(|error| format!("Detection inference failed: {error}"))?;
        let (shape, scores) = output[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("Cannot read detection result: {error}"))?;
        let dims = &**shape;
        if dims.len() != 4 || dims[2] == 0 || dims[3] == 0 {
            return Err("Unexpected detection output shape".to_owned());
        }
        let output_width = dims[3] as u32;
        let output_height = dims[2] as u32;
        let binary = scores
            .iter()
            .take((output_width * output_height) as usize)
            .map(|score| u8::from((score * 255.0).round() > 63.75))
            .collect::<Vec<_>>();
        let contours = contours(
            &dilate(&binary, output_width, output_height),
            output_width,
            output_height,
            6,
        );
        let scale_x = width as f32 / source_width as f32;
        let scale_y = height as f32 / source_height as f32;
        let boxes = contours
            .into_iter()
            .map(|rect| {
                let scaled = Rect {
                    x: rect.x.saturating_mul(width) / output_width,
                    y: rect.y.saturating_mul(height) / output_height,
                    width: (rect.width.saturating_mul(width) / output_width).max(1),
                    height: (rect.height.saturating_mul(height) / output_height).max(1),
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
        let mut recognized = Vec::new();
        for rect in boxes
            .into_iter()
            .filter(|rect| rect.width > 0 && rect.height > 0)
        {
            let crop = image::imageops::crop_imm(image, rect.x, rect.y, rect.width, rect.height)
                .to_image();
            let target_width = ((rect.width as f32 / rect.height as f32) * 48.0)
                .round()
                .max(1.0) as u32;
            let resized = image::imageops::resize(&crop, target_width, 48, FilterType::Triangle);
            let tensor = normalized_tensor(&resized, [127.5; 3], [1.0 / 127.5; 3]);
            let input = Tensor::from_array((
                [1_usize, 3, 48, target_width as usize],
                tensor.into_boxed_slice(),
            ))
            .map_err(|error| format!("Cannot build recognition tensor: {error}"))?;
            let output = self
                .recognition
                .run(ort::inputs![input])
                .map_err(|error| format!("Recognition inference failed: {error}"))?;
            let (shape, logits) = output[0]
                .try_extract_tensor::<f32>()
                .map_err(|error| format!("Cannot read recognition result: {error}"))?;
            let dims = &**shape;
            if dims.len() != 3 || dims[1] <= 0 || dims[2] <= 0 {
                return Err("Unexpected recognition output shape".to_owned());
            }
            let (text, confidence) =
                ctc_decode(logits, dims[1] as usize, dims[2] as usize, &self.dictionary);
            recognized.push(RecognizedBox {
                text,
                confidence,
                rect,
            });
        }
        recognized.sort_by(|left, right| {
            if (left.rect.y as i64 - right.rect.y as i64).unsigned_abs()
                < ((left.rect.height + right.rect.height) / 4) as u64
            {
                left.rect.x.cmp(&right.rect.x)
            } else {
                left.rect.y.cmp(&right.rect.y)
            }
        });
        Ok(join_reading_order(recognized))
    }

    pub fn recognize_region(&mut self, image: RgbaImage) -> Result<OcrText, String> {
        let enhanced = image::imageops::contrast(&image, 0.18);
        let max_side = enhanced.width().max(enhanced.height()).max(1);
        let scale = (1200 / max_side).clamp(1, 4);
        let prepared = if scale > 1 {
            image::imageops::resize(
                &enhanced,
                enhanced.width() * scale,
                enhanced.height() * scale,
                FilterType::Triangle,
            )
        } else {
            enhanced
        };
        self.recognize(&prepared)
    }
}

pub fn capture_region(region: OcrRegion) -> Result<RgbaImage, String> {
    if !region.x.is_finite()
        || !region.y.is_finite()
        || !region.width.is_finite()
        || !region.height.is_finite()
        || region.width <= 0.0
        || !region.height.is_finite()
        || region.height <= 0.0
    {
        return Err("Invalid OCR region".to_owned());
    }
    let screen = screenshots::Screen::from_point(region.x.round() as i32, region.y.round() as i32)
        .map_err(|error| format!("Cannot find OCR display: {error}"))?;
    let display = screen.display_info;
    let local_x = (region.x - display.x as f64).round() as i32;
    let local_y = (region.y - display.y as f64).round() as i32;
    let width = region.width.round().max(1.0) as u32;
    let height = region.height.round().max(1.0) as u32;

    if width == 0 || height == 0 {
        return Err("OCR region is too small after scaling".to_owned());
    }

    let rgba = screen
        .capture_area_ignore_area_check(local_x, local_y, width, height)
        .map_err(|error| format!("Cannot capture OCR region: {error}"))?;

    if rgba.width() != width || rgba.height() != height {
        return Err("Captured region has unexpected dimensions".to_owned());
    }

    Ok(rgba)
}

pub fn available_displays() -> Result<Vec<OcrDisplay>, String> {
    let displays = screenshots::Screen::all()
        .map_err(|error| format!("Cannot enumerate OCR displays: {error}"))?
        .into_iter()
        .enumerate()
        .map(|(index, screen)| {
            let display = screen.display_info;
            OcrDisplay {
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
            }
        })
        .collect::<Vec<_>>();
    Ok(displays)
}

fn ctc_decode(
    logits: &[f32],
    sequence_length: usize,
    classes: usize,
    dictionary: &[String],
) -> (String, f32) {
    let mut text = String::new();
    let mut scores = Vec::new();
    for step in 0..sequence_length {
        let mut max_score = 0.0_f32;
        let mut max_index = 0_usize;
        for (index, score) in logits[step * classes..(step + 1) * classes]
            .iter()
            .enumerate()
        {
            if *score > max_score {
                max_score = *score;
                max_index = index;
            }
        }
        if max_index != 0 {
            text.push_str(dictionary.get(max_index).map(String::as_str).unwrap_or(""));
            scores.push(max_score);
        }
    }
    let confidence = if scores.is_empty() {
        0.0
    } else {
        scores.iter().sum::<f32>() / scores.len() as f32
    };
    (text, confidence)
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
    let mut text = recognized[0].text.clone();
    let mut line = vec![&recognized[0]];
    let mut average_height = recognized[0].rect.height as f32;
    for current in recognized.iter().skip(1) {
        let previous = line.last().expect("line contains first item");
        let vertical_gap = (current.rect.y as i64 - previous.rect.y as i64).unsigned_abs() as f32;
        if vertical_gap <= average_height * 0.5 {
            text.push(' ');
            text.push_str(&current.text);
            line.push(current);
            average_height = line
                .iter()
                .map(|result| result.rect.height as f32)
                .sum::<f32>()
                / line.len() as f32;
        } else {
            text.push('\n');
            text.push_str(&current.text);
            line = vec![current];
            average_height = current.rect.height as f32;
        }
    }
    OcrText { text, confidence }
}

fn resize_for_detection(image: &RgbaImage, max_side: u32) -> (RgbaImage, u32, u32) {
    let (source_width, source_height) = image.dimensions();
    let ratio = max_side as f32 / source_width.max(source_height) as f32;
    let mut width = (source_width as f32 * ratio).floor() as u32;
    let mut height = (source_height as f32 * ratio).floor() as u32;
    if width % 32 != 0 {
        width = (width / 32).max(1) * 32;
        height = (height / 32).max(1) * 32;
    }
    (
        image::imageops::resize(image, width, height, FilterType::Triangle),
        source_width,
        source_height,
    )
}

fn normalized_tensor(image: &RgbaImage, mean: [f32; 3], norm: [f32; 3]) -> Vec<f32> {
    let (width, height) = image.dimensions();
    let plane = (width * height) as usize;
    let mut tensor = vec![0.0; plane * 3];
    for (index, pixel) in image.pixels().enumerate() {
        for channel in 0..3 {
            tensor[channel * plane + index] =
                pixel[channel] as f32 * norm[channel] - mean[channel] * norm[channel];
        }
    }
    tensor
}

fn dilate(input: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    for y in 0..height {
        for x in 0..width {
            let active = (-1_i32..=1).any(|dy| {
                (-1_i32..=1).any(|dx| {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    nx >= 0
                        && ny >= 0
                        && nx < width as i32
                        && ny < height as i32
                        && input[ny as usize * width as usize + nx as usize] != 0
                })
            });
            output[y as usize * width as usize + x as usize] = u8::from(active);
        }
    }
    output
}

fn contours(input: &[u8], width: u32, height: u32, min_area: u32) -> Vec<Rect> {
    let mut visited = vec![false; input.len()];
    let mut boxes = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let start = y as usize * width as usize + x as usize;
            if input[start] == 0 || visited[start] {
                continue;
            }
            let mut queue = std::collections::VecDeque::from([(x, y)]);
            visited[start] = true;
            let (mut min_x, mut min_y, mut max_x, mut max_y, mut area) = (x, y, x, y, 0_u32);
            while let Some((current_x, current_y)) = queue.pop_front() {
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
                            queue.push_back((nx as u32, ny as u32));
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

    #[test]
    fn bundled_ocr_assets_exist() {
        let model_dir = development_model_directory();
        assert!(model_dir.join(DETECTION_MODEL).is_file());
        assert!(model_dir.join(RECOGNITION_MODEL).is_file());
        assert!(model_dir.join(DICTIONARY).is_file());
    }

    #[test]
    fn models_open_with_the_native_onnx_runtime() {
        let status = verify_models().expect("bundled OCR models should load");
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
        let dictionary = vec![String::new(), "A".to_owned(), "B".to_owned()];
        let (text, confidence) = ctc_decode(&[0.0, 0.8, 0.1, 0.0, 0.7, 0.2], 2, 3, &dictionary);
        assert_eq!(text, "AA");
        assert!((confidence - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn region_capturer_reads_the_active_desktop() {
        let screen = screenshots::Screen::all().expect("a desktop display should exist")[0];
        let display = screen.display_info;
        let image = capture_region(OcrRegion {
            x: display.x as f64,
            y: display.y as f64,
            width: 16.0,
            height: 16.0,
        })
        .expect("desktop region should be capturable");
        assert_eq!(image.width(), 16);
    }

    #[test]
    fn native_pipeline_runs_from_capture_to_text() {
        let screen = screenshots::Screen::all().expect("a desktop display should exist")[0];
        let display = screen.display_info;
        let frame = capture_region(OcrRegion {
            x: display.x as f64,
            y: display.y as f64,
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
}
