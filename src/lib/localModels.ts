export const localModels = [
  { filename: 'MiniCPM5-2B.Q6_K.gguf', label: 'MiniCPM5-2B · Q6_K' },
] as const;
export function modelLabel(filename: string) {
  const base = filename.split(/[\\/]/).pop() ?? filename;
  return localModels.find(model => model.filename === base)?.label ?? base;
}
