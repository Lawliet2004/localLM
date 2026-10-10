export const localModels = [
  { filename: 'MiniCPM5-2B.Q6_K.gguf', label: 'MiniCPM5-2B · Q6_K' },
] as const;
export function modelLabel(filename: string) {
  const base = filename.split(/[\\/]/).pop() ?? filename;
  if (base === 'K2-Horizon-4B-Q4_K_M.gguf') return 'K2 Horizon 3.7B · Q4_K_M';
  return localModels.find(model => model.filename === base)?.label ?? base;
}
