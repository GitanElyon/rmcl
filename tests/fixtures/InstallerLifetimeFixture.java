// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

import java.nio.file.Files;
import java.nio.file.Paths;

public final class InstallerLifetimeFixture {
    public static void main(String[] args) throws Exception {
        boolean child = args.length != 0 && args[0].equals("child");
        if (!child) {
            String java = Paths.get(System.getProperty("java.home"), "bin",
                System.getProperty("os.name").startsWith("Windows") ? "java.exe" : "java").toString();
            new ProcessBuilder(java, "-cp", System.getProperty("java.class.path"),
                "InstallerLifetimeFixture", "child").inheritIO().start();
        }
        String prefix = child ? "installer-child" : "installer";
        Files.write(Paths.get(prefix + "-ready"), new byte[] {1});
        Thread.sleep(1200);
        Files.write(Paths.get(prefix + "-survived"), new byte[] {1});
    }
}
