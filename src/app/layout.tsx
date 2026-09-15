import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "QuestDrop",
  description: "Turn abandoned side-projects into tradeable quests.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
