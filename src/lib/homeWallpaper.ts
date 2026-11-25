import sakura from '../assets/wallpapers/sakura.webp';
import space from '../assets/wallpapers/space.webp';
import ocean from '../assets/wallpapers/ocean.webp';
import mountains from '../assets/wallpapers/mountains.webp';

const wallpaperAssets = { sakura, space, ocean, mountains };

export const homeWallpapers = {
  sakura: 'Cherry blossom',
  space: 'Deep space',
  ocean: 'Ocean coast',
  mountains: 'Alpine lake',
} as const;

export type HomeWallpaper = 'none' | 'custom' | keyof typeof homeWallpapers;

const maxSavedLength = 900_000;

export function validWallpaperData(value: unknown): value is string {
  return typeof value === 'string' && value.length <= maxSavedLength &&
    /^data:image\/jpeg;base64,[A-Za-z0-9+/]+={0,2}$/.test(value);
}

export function wallpaperUrl(wallpaper: HomeWallpaper, customImage: string): string | undefined {
  if (wallpaper === 'none') return undefined;
  if (wallpaper === 'custom') return validWallpaperData(customImage) ? customImage : undefined;
  return wallpaperAssets[wallpaper];
}

export async function prepareWallpaper(file: File): Promise<string> {
  if (!['image/jpeg', 'image/png', 'image/webp'].includes(file.type)) {
    throw new Error('Choose a JPEG, PNG, or WebP image.');
  }
  if (file.size > 16 * 1024 * 1024) throw new Error('Choose an image under 16 MB.');

  const source = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error('Could not read this image.'));
    reader.readAsDataURL(file);
  });
  const image = new Image();
  image.src = source;
  try {
    await image.decode();
  } catch {
    throw new Error('Could not open this image. Try another file.');
  }

  const scale = Math.min(1, 1600 / Math.max(image.naturalWidth, image.naturalHeight));
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
  const context = canvas.getContext('2d', { alpha: false });
  if (!context) throw new Error('Could not prepare the wallpaper on this device.');
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  for (const quality of [0.82, 0.65, 0.45]) {
    const data = canvas.toDataURL('image/jpeg', quality);
    if (validWallpaperData(data)) return data;
  }
  throw new Error('This image is too detailed to save. Try a smaller image.');
}
