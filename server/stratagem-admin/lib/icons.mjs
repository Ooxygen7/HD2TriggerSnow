import sharp from "sharp";
import { LIMITS } from "./constants.mjs";

const ALLOWED_TYPES = new Set(["image/svg+xml", "image/png", "image/jpeg"]);
const ALLOWED_FORMATS = new Set(["svg", "png", "jpeg"]);

function decodeUpload(upload) {
  if (!upload || typeof upload !== "object" || Array.isArray(upload)) {
    throw new TypeError("icon upload is required");
  }
  if (!ALLOWED_TYPES.has(upload.mediaType)) throw new TypeError("icon type is unsupported");
  if (typeof upload.base64 !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(upload.base64)) {
    throw new TypeError("icon data is invalid");
  }
  const bytes = Buffer.from(upload.base64, "base64");
  if (bytes.length === 0 || bytes.length > LIMITS.sourceIconBytes) {
    throw new TypeError("icon must be no larger than 1 MiB");
  }
  return bytes;
}

export async function normalizeUploadedIcon(upload) {
  const source = decodeUpload(upload);
  const image = sharp(source, {
    failOn: "warning",
    limitInputPixels: 16 * 1024 * 1024,
    limitInputChannels: 4,
    pages: 1,
    animated: false,
    density: 144,
  });
  const metadata = await image.metadata();
  if (!ALLOWED_FORMATS.has(metadata.format)) throw new TypeError("decoded icon format is unsupported");
  if (metadata.mediaType && metadata.mediaType !== upload.mediaType) {
    throw new TypeError("icon media type does not match its contents");
  }
  const png = await image
    .rotate()
    .resize(208, 208, {
      fit: "contain",
      withoutEnlargement: false,
      background: { r: 0, g: 0, b: 0, alpha: 0 },
    })
    .png({ compressionLevel: 9, adaptiveFiltering: true, palette: true })
    .toBuffer();

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" data-hd2-normalized-icon="1"><path fill="#011419" fill-opacity=".75" d="M0 0h256v256H0z"/><image x="24" y="24" width="208" height="208" preserveAspectRatio="xMidYMid meet" href="data:image/png;base64,${png.toString("base64")}"/><path fill="#5abeda" fill-rule="evenodd" d="M0 0h256v256H0zM14 14v228h228V14H14z"/></svg>`;
  const bytes = Buffer.from(svg, "utf8");
  if (bytes.length > LIMITS.normalizedIconBytes) throw new TypeError("normalized icon exceeds the size limit");
  return {
    kind: "data",
    mediaType: "image/svg+xml",
    base64: bytes.toString("base64"),
  };
}
