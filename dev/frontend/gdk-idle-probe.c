/* Standalone diagnostic: no MiRelay, image decoder, window, or file transfer.
 * cc -O2 -g dev/frontend/gdk-idle-probe.c -o target/gdk-idle-probe $(pkg-config --cflags --libs gtk4)
 * Run on an isolated display: compare "copy" and "convert", then GDK_DISABLE=threads.
 */
#include <gtk/gtk.h>
#include <sys/resource.h>

static double cpu_seconds (void)
{
  struct rusage usage;
  g_assert_cmpint (getrusage (RUSAGE_SELF, &usage), ==, 0);
  return usage.ru_utime.tv_sec + usage.ru_utime.tv_usec / 1e6
       + usage.ru_stime.tv_sec + usage.ru_stime.tv_usec / 1e6;
}

static gboolean stop_loop (gpointer data)
{
  g_main_loop_quit (data);
  return G_SOURCE_REMOVE;
}

static void sample (const char *mode, int stage)
{
  GMainLoop *loop = g_main_loop_new (NULL, FALSE);
  double before = cpu_seconds ();
  gint64 start = g_get_monotonic_time ();
  g_timeout_add (2000, stop_loop, loop);
  g_main_loop_run (loop);
  double elapsed = (g_get_monotonic_time () - start) / 1e6;
  g_print ("GDK_IDLE mode=%s stage=%d seconds=%.3f cpu_one_core=%.3f%%\n",
           mode, stage, elapsed, (cpu_seconds () - before) / elapsed * 100);
  g_main_loop_unref (loop);
}

int main (int argc, char **argv)
{
  if (argc != 2 || (g_strcmp0 (argv[1], "copy") && g_strcmp0 (argv[1], "convert")))
    { g_printerr ("usage: gdk-idle-probe copy|convert\n"); return 2; }
  gboolean convert = g_str_equal (argv[1], "convert");
  gtk_init ();
  g_print ("GDK_IDLE gtk=%u.%u.%u glib=%u.%u.%u mode=%s\n",
           gtk_get_major_version (), gtk_get_minor_version (), gtk_get_micro_version (),
           glib_major_version, glib_minor_version, glib_micro_version, argv[1]);
  sample (argv[1], -1);
  const int size = 256;
  const int channels = convert ? 3 : 4;
  guchar *input = g_malloc (size * size * channels);
  memset (input, 128, size * size * channels);
  GBytes *bytes = g_bytes_new_take (input, size * size * channels);
  guchar *output = g_malloc (size * size * 4);
  gint64 start = g_get_monotonic_time ();
  for (int i = 0; i < 400; i++)
    {
      GdkTexture *texture = gdk_memory_texture_new (size, size,
          convert ? GDK_MEMORY_R8G8B8 : GDK_MEMORY_DEFAULT, bytes, size * channels);
      gdk_texture_download (texture, output, size * 4);
      g_assert_cmpuint (output[0], ==, 128);
      g_object_unref (texture);
    }
  g_print ("GDK_IDLE work_ms=%.3f textures=400\n", (g_get_monotonic_time () - start) / 1000.0);
  g_free (output);
  g_bytes_unref (bytes);
  for (int stage = 0; stage < 4; stage++) sample (argv[1], stage);
  return 0;
}
