class Quilt {
    private static native String install(String domain, String uri);

    static {
        System.loadLibrary("quilt_rs");
    }

    public static void main(String[] args) {
        String output = Quilt.install("./TEST", "quilt+s3://fiskus-us-east-1#package=scale/100u");
        System.out.println(output);
    }
}

