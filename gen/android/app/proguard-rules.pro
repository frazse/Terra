# Add project specific ProGuard rules here.

-keep class dev.frazse.terra.BleManager { *; }
-keepclassmembers class dev.frazse.terra.BleManager { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}
