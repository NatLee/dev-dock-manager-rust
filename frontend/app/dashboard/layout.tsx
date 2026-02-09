"use client";

import { useEffect, useState } from "react";
import { usePathname, useRouter } from "next/navigation";
import { useAuth } from "@/contexts/AuthContext";
import { Navbar } from "@/components/layout/Navbar";
import { Toaster } from "sonner";

export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const { token, isReady, ensureAuth } = useAuth();
  const [checked, setChecked] = useState(false);
  const isConsoleRoute = pathname?.startsWith("/dashboard/console") ?? false;

  useEffect(() => {
    if (!isReady) return;
    const run = async () => {
      const ok = await ensureAuth();
      setChecked(true);
      if (!ok) router.replace("/login");
    };
    run();
  }, [isReady, ensureAuth, router]);

  if (!isReady || !checked || !token) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-surface">
        <div className="text-text-muted">Loading...</div>
      </div>
    );
  }

  return (
    <>
      <Navbar />
      <main
        className={
          isConsoleRoute
            ? "flex h-[calc(100vh-3.5rem)] max-h-[calc(100vh-3.5rem)] flex-col overflow-hidden px-4 py-4"
            : "container mx-auto mt-5 max-w-7xl bg-background px-4 py-6"
        }
      >
        <div id="toast-container" />
        {children}
      </main>
      <Toaster position="top-right" richColors />
    </>
  );
}
