export interface FormatMeta {
  shortLabel: string;
  title: string;
  accent: string;
  iconSvg: string;
}

const STROKE_ATTRS =
  'fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"';

function svg(body: string, size = 20): string {
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true">${body}</svg>`;
}

export const UI_ICONS = {
  more: svg(
    `<g ${STROKE_ATTRS}><circle cx="6" cy="12" r="1.2" fill="currentColor"/><circle cx="12" cy="12" r="1.2" fill="currentColor"/><circle cx="18" cy="12" r="1.2" fill="currentColor"/></g>`,
    20,
  ),
  back: svg(
    `<g ${STROKE_ATTRS}><path d="M15 6l-6 6 6 6"/><path d="M9 12h9"/></g>`,
    20,
  ),
  convert: svg(
    `<g ${STROKE_ATTRS}><path d="M17 3l4 4-4 4"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><path d="M7 21l-4-4 4-4"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/></g>`,
    14,
  ),
  newFile: svg(
    `<g ${STROKE_ATTRS}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6"/><path d="M12 11v6"/><path d="M9 14h6"/></g>`,
    14,
  ),
  folder: svg(
    `<g ${STROKE_ATTRS}><path d="M3 6a2 2 0 0 1 2-2h4.2a2 2 0 0 1 1.5.7l1.3 1.6a2 2 0 0 0 1.5.7H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></g>`,
    20,
  ),
  image: svg(
    `<g ${STROKE_ATTRS}><rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="8.5" cy="9.5" r="1.5"/><path d="M21 16l-5-5-5 5-2-2-6 6"/></g>`,
    20,
  ),
  pdf: svg(
    `<g ${STROKE_ATTRS}><path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z"/><path d="M14 2v5h5"/><path d="M8.5 13h7"/><path d="M8.5 16.5h5"/></g>`,
    20,
  ),
  combinePdf: svg(
    `<g ${STROKE_ATTRS}><rect x="7" y="4" width="12" height="15" rx="2"/><path d="M5 8v11a2 2 0 0 0 2 2h9"/><path d="M13 9v5"/><path d="M10.5 11.5h5"/></g>`,
    20,
  ),
  document: svg(
    `<g ${STROKE_ATTRS}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6"/><path d="M8 13h8"/><path d="M8 17h6"/></g>`,
    20,
  ),
  markdown: svg(
    `<g ${STROKE_ATTRS}><rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M7 15V9l2.5 3L12 9v6"/><path d="M16.5 9v6"/><path d="M14.5 13l2 2 2-2"/></g>`,
    20,
  ),
  spreadsheet: svg(
    `<g ${STROKE_ATTRS}><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 10h18"/><path d="M3 15h18"/><path d="M9 4v16"/><path d="M15 4v16"/></g>`,
    20,
  ),
  presentation: svg(
    `<g ${STROKE_ATTRS}><rect x="3" y="4" width="18" height="13" rx="2"/><path d="M12 17v4"/><path d="M8 21h8"/><path d="M8 10h8"/></g>`,
    20,
  ),
  code: svg(
    `<g ${STROKE_ATTRS}><path d="M8 8l-4 4 4 4"/><path d="M16 8l4 4-4 4"/><path d="M13.5 5l-3 14"/></g>`,
    20,
  ),
  data: svg(
    `<g ${STROKE_ATTRS}><path d="M8 4H6a2 2 0 0 0-2 2v3a2 2 0 0 1-1.5 2A2 2 0 0 1 4 13v3a2 2 0 0 0 2 2h2"/><path d="M16 4h2a2 2 0 0 1 2 2v3a2 2 0 0 0 1.5 2A2 2 0 0 0 20 13v3a2 2 0 0 1-2 2h-2"/></g>`,
    20,
  ),
  audio: svg(
    `<g ${STROKE_ATTRS}><path d="M4 10v4"/><path d="M8 7v10"/><path d="M12 4v16"/><path d="M16 7v10"/><path d="M20 10v4"/></g>`,
    20,
  ),
  video: svg(
    `<g ${STROKE_ATTRS}><rect x="3" y="5" width="14" height="14" rx="2.5"/><path d="M17 10l4-2.5v9L17 14z"/></g>`,
    20,
  ),
  clipboard: svg(
    `<g ${STROKE_ATTRS}><rect x="5" y="4" width="14" height="17" rx="2.5"/><path d="M9 4.5h6a1 1 0 0 0 1-1V3a1 1 0 0 0-1-1H9a1 1 0 0 0-1 1v.5a1 1 0 0 0 1 1z"/><path d="M9 11h6"/><path d="M9 15h4"/></g>`,
    20,
  ),
  book: svg(
    `<g ${STROKE_ATTRS}><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></g>`,
    20,
  ),
  template: svg(
    `<g ${STROKE_ATTRS}><path d="M12 3l1.9 4.2 4.6.6-3.4 3.2.8 4.6-3.9-2.2-3.9 2.2.8-4.6L5.5 7.8l4.6-.6z"/></g>`,
    20,
  ),
  drop: svg(
    `<g ${STROKE_ATTRS}><path d="M12 4v11"/><path d="M7.5 10.5L12 15l4.5-4.5"/><path d="M4 17v1a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-1"/></g>`,
    22,
  ),
  settings: svg(
    `<g ${STROKE_ATTRS}><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.79 1 1.51 1H21a2 2 0 1 1 0 4h-.09c-.72 0-1.25.4-1.51 1z"/></g>`,
    16,
  ),
  keyboard: svg(
    `<g ${STROKE_ATTRS}><rect x="2" y="6" width="20" height="12" rx="2.5"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/></g>`,
    16,
  ),
  sliders: svg(
    `<g ${STROKE_ATTRS}><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"/></g>`,
    16,
  ),
  shield: svg(
    `<g ${STROKE_ATTRS}><path d="M12 2l7 3.5v6c0 5-3 8.8-7 10.5-4-1.7-7-5.5-7-10.5v-6z"/><path d="M9.5 12l1.8 1.8 3.7-3.8"/></g>`,
    16,
  ),
  info: svg(
    `<g ${STROKE_ATTRS}><circle cx="12" cy="12" r="9"/><path d="M12 16v-4"/><path d="M12 8h.01"/></g>`,
    16,
  ),
  finder: svg(
    `<g ${STROKE_ATTRS}><rect x="3" y="3" width="18" height="18" rx="4"/><path d="M12 3c-1.5 3.5-1.5 7 0 9h2c0 3-1 6-2 9"/><path d="M8 10v1M16 10v1"/><path d="M7.5 15.5c1.3 1.2 3 1.8 4.5 1.8s3.2-.6 4.5-1.8"/></g>`,
    16,
  ),
  refresh: svg(
    `<g ${STROKE_ATTRS}><path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v6h-6"/></g>`,
    15,
  ),
  trash: svg(
    `<g ${STROKE_ATTRS}><path d="M4 7h16"/><path d="M10 11v6M14 11v6"/><path d="M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12"/><path d="M9 7V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"/></g>`,
    14,
  ),
  removeBg: svg(
    `<g ${STROKE_ATTRS}><path d="M12 3l1.4 3.4L17 7.8l-3.6 1.4L12 12.6l-1.4-3.4L7 7.8l3.6-1.4z"/><circle cx="15" cy="15" r="5"/><path d="M12 18l6-6"/></g>`,
    18,
  ),
  optimizePdf: svg(
    `<g ${STROKE_ATTRS}><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/><path d="M9 14l3-3 3 3"/><path d="M12 11v6"/></g>`,
    18,
  ),
  zip: svg(
    `<g ${STROKE_ATTRS}><rect x="4" y="4" width="16" height="16" rx="3"/><path d="M11 4v16"/><path d="M11 8h2.5M11 11h2.5M11 14h2.5"/></g>`,
    18,
  ),
  unzip: svg(
    `<g ${STROKE_ATTRS}><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M12 15v-5"/><path d="M9.5 12.5L12 10l2.5 2.5"/></g>`,
    18,
  ),
};

export function getFormatMeta(format: string): FormatMeta {
  const key = format.trim().toUpperCase();
  switch (key) {
    case "REMOVE_BG":
      return {
        shortLabel: "No BG",
        title: "Remove BG",
        accent: "#ec4899",
        iconSvg: UI_ICONS.removeBg,
      };
    case "OPTIMIZE_PDF":
      return {
        shortLabel: "Opt PDF",
        title: "Optimize PDF",
        accent: "#f59e0b",
        iconSvg: UI_ICONS.optimizePdf,
      };
    case "ZIP":
      return {
        shortLabel: "ZIP",
        title: "Zip Archive",
        accent: "#eab308",
        iconSvg: UI_ICONS.zip,
      };
    case "UNZIP":
      return {
        shortLabel: "UNZIP",
        title: "Unzip Folder",
        accent: "#14b8a6",
        iconSvg: UI_ICONS.unzip,
      };
    case "COMBINE_PDF":
      return {
        shortLabel: "PDF+",
        title: "Combine into PDF",
        accent: "#ff6b4a",
        iconSvg: UI_ICONS.combinePdf,
      };
    case "MERGE_PDF":
      return {
        shortLabel: "MERGE",
        title: "Merge PDFs",
        accent: "#ff5c5c",
        iconSvg: UI_ICONS.combinePdf,
      };
    case "PNG":
    case "JPG":
    case "JPEG":
    case "WEBP":
    case "HEIC":
    case "TIFF":
    case "GIF":
    case "BMP":
      return {
        shortLabel: key,
        title: `Convert to ${key}`,
        accent: "#38bdf8",
        iconSvg: UI_ICONS.image,
      };
    case "PDF":
      return {
        shortLabel: "PDF",
        title: "Convert to PDF",
        accent: "#fb7185",
        iconSvg: UI_ICONS.pdf,
      };
    case "MD":
      return {
        shortLabel: "MD",
        title: "Convert to Markdown",
        accent: "#a78bfa",
        iconSvg: UI_ICONS.markdown,
      };
    case "HTML":
      return {
        shortLabel: "HTML",
        title: "Convert to HTML",
        accent: "#f97316",
        iconSvg: UI_ICONS.code,
      };
    case "EPUB":
    case "FB2":
      return {
        shortLabel: key,
        title: `Convert to ${key}`,
        accent: "#c084fc",
        iconSvg: UI_ICONS.book,
      };
    case "DOCX":
    case "TXT":
    case "RTF":
    case "ODT":
    case "RST":
      return {
        shortLabel: key,
        title: `Convert to ${key}`,
        accent: "#60a5fa",
        iconSvg: UI_ICONS.document,
      };
    case "XLSX":
    case "CSV":
      return {
        shortLabel: key,
        title: `Convert to ${key}`,
        accent: "#34d399",
        iconSvg: UI_ICONS.spreadsheet,
      };
    case "PPTX":
      return {
        shortLabel: "PPTX",
        title: "Convert to PPTX",
        accent: "#fb923c",
        iconSvg: UI_ICONS.presentation,
      };
    case "JSON":
    case "YAML":
    case "TOML":
    case "XML":
      return {
        shortLabel: key,
        title: `Convert to ${key}`,
        accent: "#2dd4bf",
        iconSvg: UI_ICONS.data,
      };
    case "MP3":
    case "M4A":
    case "WAV":
    case "FLAC":
    case "AIFF":
    case "ALAC":
    case "OGG":
    case "OPUS":
    case "AAC":
      return {
        shortLabel: key,
        title: `Convert to ${key}`,
        accent: "#f472b6",
        iconSvg: UI_ICONS.audio,
      };
    case "MP4":
    case "MOV":
    case "MKV":
    case "WEBM":
    case "AVI":
    case "M4V":
      return {
        shortLabel: key,
        title: `Convert to ${key}`,
        accent: "#818cf8",
        iconSvg: UI_ICONS.video,
      };
    default:
      return {
        shortLabel: key.slice(0, 6),
        title: `Convert to ${key}`,
        accent: "#38bdf8",
        iconSvg: UI_ICONS.document,
      };
  }
}

export function getNewKindMeta(
  id: string,
  name: string,
  extension: string | null,
): { shortLabel: string; accent: string; iconSvg: string } {
  if (id.startsWith("template:")) {
    const ext = extension ? `.${extension.toUpperCase()}` : name.slice(0, 6).toUpperCase();
    return {
      shortLabel: ext.slice(0, 6),
      accent: "#fbbf24",
      iconSvg: UI_ICONS.template,
    };
  }

  switch (id) {
    case "folder":
      return { shortLabel: "Folder", accent: "#38bdf8", iconSvg: UI_ICONS.folder };
    case "txt":
      return { shortLabel: "TXT", accent: "#93c5fd", iconSvg: UI_ICONS.document };
    case "md":
      return { shortLabel: "MD", accent: "#c4b5fd", iconSvg: UI_ICONS.markdown };
    case "rtf":
      return { shortLabel: "RTF", accent: "#60a5fa", iconSvg: UI_ICONS.document };
    case "docx":
      return { shortLabel: "DOCX", accent: "#3b82f6", iconSvg: UI_ICONS.document };
    case "xlsx":
      return { shortLabel: "XLSX", accent: "#34d399", iconSvg: UI_ICONS.spreadsheet };
    case "pptx":
      return { shortLabel: "PPTX", accent: "#fb923c", iconSvg: UI_ICONS.presentation };
    case "html":
      return { shortLabel: "HTML", accent: "#f97316", iconSvg: UI_ICONS.code };
    case "clipboard":
      return { shortLabel: "Clip", accent: "#f472b6", iconSvg: UI_ICONS.clipboard };
    case "json":
      return { shortLabel: "JSON", accent: "#2dd4bf", iconSvg: UI_ICONS.data };
    case "csv":
      return { shortLabel: "CSV", accent: "#4ade80", iconSvg: UI_ICONS.spreadsheet };
    case "sh":
      return { shortLabel: "SH", accent: "#a3e635", iconSvg: UI_ICONS.code };
    case "py":
      return { shortLabel: "PY", accent: "#facc15", iconSvg: UI_ICONS.code };
    case "swift":
      return { shortLabel: "Swift", accent: "#fb7185", iconSvg: UI_ICONS.code };
    case "pages":
      return { shortLabel: "Pages", accent: "#fb923c", iconSvg: UI_ICONS.document };
    case "numbers":
      return { shortLabel: "Numbers", accent: "#4ade80", iconSvg: UI_ICONS.spreadsheet };
    case "keynote":
      return { shortLabel: "Keynote", accent: "#38bdf8", iconSvg: UI_ICONS.presentation };
    default:
      return {
        shortLabel: (extension ?? name).slice(0, 6),
        accent: "#60a5fa",
        iconSvg: UI_ICONS.document,
      };
  }
}
