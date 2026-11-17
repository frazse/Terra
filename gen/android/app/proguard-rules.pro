# Add project specific ProGuard rules here.

-keep class dev.frazse.terra.BleManager { *; }
-keepclassmembers class dev.frazse.terra.BleManager { *; }
-keep class dev.frazse.terra.MainActivity$Companion { *; }
-keepclassmembers class dev.frazse.terra.MainActivity {
    public static *;
}
-keepclasseswithmembernames class * {
    native <methods>;
}
