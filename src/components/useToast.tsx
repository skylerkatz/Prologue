import { useCallback, useEffect, useRef, useState } from "react";

export interface ToastMessage {
  text: string;
  error: boolean;
}

/**
 * A transient status message that dismisses itself after `ms`. Showing a new
 * message replaces the current one and restarts the timer; the timer is
 * cleared on unmount. `showToast` is stable across renders.
 */
export function useToast(): {
  toast: ToastMessage | null;
  showToast: (text: string, ms: number, error?: boolean) => void;
} {
  const [toast, setToast] = useState<ToastMessage | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const showToast = useCallback((text: string, ms: number, error = false) => {
    window.clearTimeout(timer.current);
    setToast({ text, error });
    timer.current = window.setTimeout(() => setToast(null), ms);
  }, []);

  return { toast, showToast };
}

export function Toast({
  text,
  error = false,
}: {
  text: string;
  error?: boolean;
}) {
  return (
    <div className={error ? "copy-toast error" : "copy-toast"} role="status">
      {text}
    </div>
  );
}
