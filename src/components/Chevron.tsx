/** Disclosure chevron for the file-toggle buttons: a crisp stroked SVG in
 * place of the unicode caret (whose optical size varies by font). Points
 * right when collapsed and rotates down when expanded. */
export function Chevron({ expanded }: { expanded: boolean }) {
  return (
    <svg
      className={`chevron${expanded ? " chevron-expanded" : ""}`}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      aria-hidden="true"
    >
      <path
        d="M4.25 2.5 L8.25 6 L4.25 9.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
