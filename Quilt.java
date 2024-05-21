class Quilt {
    // TODO: message is optional
    // TODO: optional meta
    private static native String commit(String domain, String namespace, String message);

    private static native String install(String domain, String uri);

    private static native String push(String domain, String namespace);

    static {
        System.loadLibrary("quilt_rs");
    }

    public static void main(String[] args) {
        String domain_path = "./TEST";
        String uri = "quilt+s3://fiskus-us-east-1#package=scale/100u";

        String installed_package_path = Quilt.install(domain_path, uri);
        System.out.println(String.format("Package installed to %s", installed_package_path));
    }
}

