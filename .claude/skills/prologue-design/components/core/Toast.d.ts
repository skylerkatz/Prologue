export interface ToastProps {
  error?: boolean;
  /** position:fixed bottom-center (default) or inline for demos. */
  fixed?: boolean;
  children: React.ReactNode;
}
