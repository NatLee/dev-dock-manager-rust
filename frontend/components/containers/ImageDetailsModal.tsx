"use client";

import type { Image } from "@/types/api";

function formatSize(mb: number): string {
  if (mb >= 1024) return (mb / 1024).toFixed(2) + " GB";
  return mb.toFixed(2) + " MB";
}

type Props = {
  image: Image | null;
  onClose: () => void;
  onCreateClick?: () => void;
};

export function ImageDetailsModal({ image, onClose, onCreateClick }: Props) {
  if (!image) return null;

  const tags = image.tags ?? (image.name ? [image.name] : []);
  const displayName = image.name ?? image.short_id;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        aria-hidden
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-modal
        aria-labelledby="image-details-title"
        className="relative w-full max-w-lg rounded-2xl border border-border bg-background-elevated shadow-xl"
      >
        <div className="flex items-center justify-between gap-4 rounded-t-2xl border-b border-border bg-surface px-4 py-3">
          <h2
            id="image-details-title"
            className="text-base font-semibold text-text"
          >
            Image Details
          </h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg p-1.5 text-text-muted transition-colors hover:bg-border hover:text-text focus:outline-none focus:ring-2 focus:ring-primary/20"
            aria-label="Close"
          >
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
        <div className="max-h-[70vh] overflow-y-auto p-4">
          <dl className="space-y-3 text-sm">
            <div>
              <dt className="text-xs font-medium uppercase tracking-wider text-text-muted">Name</dt>
              <dd className="mt-0.5 font-mono text-text">{displayName}</dd>
            </div>
            <div>
              <dt className="text-xs font-medium uppercase tracking-wider text-text-muted">Short ID</dt>
              <dd className="mt-0.5 font-mono text-text">{image.short_id}</dd>
            </div>
            <div>
              <dt className="text-xs font-medium uppercase tracking-wider text-text-muted">Full ID</dt>
              <dd className="mt-0.5 break-all font-mono text-xs text-text">{image.id}</dd>
            </div>
            <div>
              <dt className="text-xs font-medium uppercase tracking-wider text-text-muted">Size</dt>
              <dd className="mt-0.5 font-mono text-text">{formatSize(image.size)}</dd>
            </div>
            {tags.length > 0 && (
              <div>
                <dt className="text-xs font-medium uppercase tracking-wider text-text-muted">Tags</dt>
                <dd className="mt-1 flex flex-wrap gap-1.5">
                  {tags.map((tag) => (
                    <span
                      key={tag}
                      className="inline-flex items-center rounded bg-primary/10 px-2 py-0.5 font-mono text-xs text-primary"
                    >
                      {tag}
                    </span>
                  ))}
                </dd>
              </div>
            )}
          </dl>
          {onCreateClick && (
            <div className="mt-4 border-t border-border pt-4">
              <button
                type="button"
                onClick={onCreateClick}
                className="w-full rounded-xl bg-primary py-2.5 text-sm font-medium text-white transition-colors hover:bg-primary-hover"
              >
                Create container from this image
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
