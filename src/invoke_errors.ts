export interface InvokeError { code: string; detail: string }

export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error;
  if (error && typeof error === 'object' && 'detail' in error) return String(error.detail);
  if (error && typeof error === 'object' && 'message' in error) return String(error.message);
  return 'Не удалось выполнить действие';
}
