"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState } from "react";
import { useAuth } from "@/contexts/AuthContext";

const navLinks = [
  { href: "/dashboard/containers", label: "Containers" },
];

export function Navbar() {
  const pathname = usePathname();
  const { logout } = useAuth();
  const [open, setOpen] = useState(false);

  return (
    <nav className="sticky top-0 z-40 border-b border-border bg-surface-nav shadow-sm">
      <div className="mx-auto flex h-14 max-w-6xl items-center justify-between px-4 sm:px-6">
        <Link
          href="/dashboard/containers"
          className="flex items-center gap-2 text-text no-underline hover:text-primary"
        >
          <span className="text-lg font-semibold tracking-tight">
            Dev Dock Manager
          </span>
        </Link>

        <div className="hidden items-center gap-1 sm:flex">
          {navLinks.map(({ href, label }) => (
            <Link
              key={href}
              href={href}
              className={`rounded-lg px-3 py-2 text-sm font-medium transition-colors ${
                pathname === href
                  ? "bg-primary/15 text-primary"
                  : "text-text-muted hover:bg-border/50 hover:text-primary"
              }`}
            >
              {label}
            </Link>
          ))}
          <div className="ml-2 h-4 w-px bg-border" aria-hidden />
          <button
            type="button"
            onClick={logout}
            className="rounded-lg px-3 py-2 text-sm font-medium text-text-muted transition-colors hover:bg-border/50 hover:text-primary"
          >
            Logout
          </button>
        </div>

        <button
          type="button"
          className="rounded-lg p-2 text-text-muted transition-colors hover:bg-border/50 hover:text-primary sm:hidden"
          onClick={() => setOpen((o) => !o)}
          aria-label="Toggle menu"
          aria-expanded={open}
        >
          <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            {open ? (
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            ) : (
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
            )}
          </svg>
        </button>
      </div>

      {open && (
        <div className="border-t border-border bg-surface-nav py-2 sm:hidden">
          {navLinks.map(({ href, label }) => (
            <Link
              key={href}
              href={href}
              className="block px-4 py-2.5 text-sm font-medium text-text transition-colors hover:bg-border/50 hover:text-primary"
              onClick={() => setOpen(false)}
            >
              {label}
            </Link>
          ))}
          <button
            type="button"
            className="block w-full px-4 py-2.5 text-left text-sm font-medium text-text-muted transition-colors hover:bg-border/50 hover:text-primary"
            onClick={() => {
              setOpen(false);
              logout();
            }}
          >
            Logout
          </button>
        </div>
      )}
    </nav>
  );
}
