import { useCallback, useState } from "react";

export interface Toast {
  id: number;
  text: string;
}

/** Short-lived messages (at most four on screen, each for six seconds). */
export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toast = useCallback((text: string) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t.slice(-3), { id, text }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 6000);
  }, []);
  return { toasts, toast };
}
