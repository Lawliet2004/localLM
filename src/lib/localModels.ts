export const localModels = [
  { filename: 'MiniCPM5-2B.Q6_K.gguf', label: 'MiniCPM5-2B · Q6_K' },
  { filename: 'Ternary-Bonsai-2-9B-PQ2_0-MTP-Q8_0.gguf', label: 'Bonsai 2 9B · PQ2_0 + MTP' },
] as const;
export function modelLabel(filename: string) {
  const base = filename.split(/[\\/]/).pop() ?? filename;
  return localModels.find(model => model.filename === base)?.label ?? base;
}
