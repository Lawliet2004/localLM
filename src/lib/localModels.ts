export const localModels = [
  { filename: 'MiniCPM5-2B.Q6_K.gguf', label: 'MiniCPM5-2B · Q6_K' },
  { filename: 'Ternary-Bonsai-8B-Q2_0.gguf', label: 'Ternary Bonsai 8B · Q2_0' },
  { filename: 'ZAYA1-8B-Q4_K_M.gguf', label: 'ZAYA1-8B · Q4_K_M' },
] as const;
export const bonsaiFilename = localModels[1].filename;
export const zaya1Filename = localModels[2].filename;
export function modelLabel(filename: string) {
  const base = filename.split(/[\\/]/).pop() ?? filename;
  return localModels.find(model => model.filename === base)?.label ?? base;
}
