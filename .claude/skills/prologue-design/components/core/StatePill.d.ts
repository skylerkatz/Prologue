export interface StatePillProps {
  kind: "resolved" | "dismissed" | "readonly" | "changed";
  title?: string;
}
